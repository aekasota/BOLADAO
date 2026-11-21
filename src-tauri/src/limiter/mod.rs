use base64::Engine;
use parking_lot::Mutex;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::VecDeque;
use std::io;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

const CHUNK_BYTES: usize = 16 * 1024;
const MAX_BURST_BYTES: f64 = 256.0 * 1024.0;
const OUTBOUND_SO_RCVBUF: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BandwidthProfile {
    Unlimited,
    Balanced,
    Light,
    Custom,
}

impl Default for BandwidthProfile {
    fn default() -> Self {
        Self::Unlimited
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimiterStatsPayload {
    pub speed_bps: u64,
    pub limit_bps: u64,
    pub profile: BandwidthProfile,
    pub peak_7d_bps: u64,
    pub sparkline_60s: Vec<u64>,
    pub proxy_port: Option<u16>,
    pub fallback_mode: bool,
    pub game_mode_override_active: bool,
}

struct BucketInner {
    rate_bps: u64,
    tokens: f64,
    last_refill: Instant,
}

pub struct TokenBucket {
    inner: Mutex<BucketInner>,
    notify: Notify,
    bytes_in_window: AtomicU64,
    total_bytes: AtomicU64,
}

impl TokenBucket {
    pub fn new(initial_rate_bps: u64) -> Arc<Self> {
        let initial_tokens = Self::burst_for(initial_rate_bps);
        Arc::new(Self {
            inner: Mutex::new(BucketInner {
                rate_bps: initial_rate_bps,
                tokens: initial_tokens,
                last_refill: Instant::now(),
            }),
            notify: Notify::new(),
            bytes_in_window: AtomicU64::new(0),
            total_bytes: AtomicU64::new(0),
        })
    }

    fn burst_for(rate_bps: u64) -> f64 {
        if rate_bps == 0 {
            MAX_BURST_BYTES
        } else {
            // Keep burst <= 125ms of bandwidth (capped at 256 KB) for tight latency control
            ((rate_bps as f64) * 0.125).clamp(CHUNK_BYTES as f64, MAX_BURST_BYTES)
        }
    }

    pub fn set_rate_bps(&self, rate_bps: u64) {
        {
            let mut g = self.inner.lock();
            let now = Instant::now();
            if g.rate_bps > 0 {
                let elapsed = now.duration_since(g.last_refill).as_secs_f64();
                let burst = Self::burst_for(g.rate_bps);
                g.tokens = (g.tokens + elapsed * g.rate_bps as f64).min(burst);
            }
            g.rate_bps = rate_bps;
            let new_burst = Self::burst_for(rate_bps);
            g.tokens = g.tokens.min(new_burst);
            g.last_refill = now;
        }
        self.notify.notify_waiters();
    }

    pub fn rate_bps(&self) -> u64 {
        self.inner.lock().rate_bps
    }

    pub fn record_bytes(&self, n: usize) {
        self.bytes_in_window.fetch_add(n as u64, Ordering::Relaxed);
        self.total_bytes.fetch_add(n as u64, Ordering::Relaxed);
    }

    pub fn drain_window_bytes(&self) -> u64 {
        self.bytes_in_window.swap(0, Ordering::Relaxed)
    }

    /// Acquires `amount` bytes from the shared token bucket, sleeping smoothly if needed
    /// and waking up immediately if `set_rate_bps` is called concurrently.
    pub async fn acquire(&self, amount: usize) {
        let needed = amount as f64;
        loop {
            let notified = self.notify.notified();
            let wait_dur = {
                let mut g = self.inner.lock();
                if g.rate_bps == 0 {
                    return;
                }
                let now = Instant::now();
                let elapsed = now.duration_since(g.last_refill).as_secs_f64();
                let burst = Self::burst_for(g.rate_bps);
                g.tokens = (g.tokens + elapsed * g.rate_bps as f64).min(burst);
                g.last_refill = now;

                if g.tokens >= needed {
                    g.tokens -= needed;
                    return;
                }
                let deficit = needed - g.tokens;
                let secs = (deficit / g.rate_bps as f64).min(0.25);
                Duration::from_secs_f64(secs.max(0.002))
            };

            tokio::select! {
                _ = tokio::time::sleep(wait_dur) => {}
                _ = notified => {}
            }
        }
    }
}

struct MetricsState {
    profile: BandwidthProfile,
    configured_custom_bps: u64,
    light_bps: u64,
    peak_7d_bps: u64,
    game_mode_active: bool,
    game_profile: BandwidthProfile,
    game_custom_bps: u64,
    ema_speed_bps: f64,
    sparkline_60s: VecDeque<u64>,
    ticks_mod_4: u8,
}

pub struct BandwidthLimiter {
    bucket: Arc<TokenBucket>,
    port: Option<u16>,
    username: String,
    password: String,
    metrics: Mutex<MetricsState>,
}

impl BandwidthLimiter {
    /// Starts the local loopback HTTP/CONNECT proxy on `127.0.0.1:0` with random session credentials.
    pub async fn start(
        initial_profile: BandwidthProfile,
        custom_bps: u64,
        light_bps: u64,
        peak_7d_bps: u64,
    ) -> Arc<Self> {
        let mut rng_bytes = [0u8; 24];
        rand::thread_rng().fill_bytes(&mut rng_bytes);
        let username = format!("b_{}", hex::encode(&rng_bytes[..8]));
        let password = hex::encode(&rng_bytes[8..]);
        let b64 = base64::engine::general_purpose::STANDARD
            .encode(format!("{username}:{password}"));
        let expected_auth_header = format!("Basic {b64}");

        let effective = compute_effective_limit(initial_profile, custom_bps, light_bps, peak_7d_bps);
        let bucket = TokenBucket::new(effective);

        let listener_res = TcpListener::bind("127.0.0.1:0").await;
        let port = listener_res
            .as_ref()
            .ok()
            .and_then(|l| l.local_addr().ok())
            .map(|a| a.port());

        let mut spark = VecDeque::with_capacity(60);
        for _ in 0..60 {
            spark.push_back(0);
        }

        let limiter = Arc::new(Self {
            bucket: bucket.clone(),
            port,
            username,
            password,
            metrics: Mutex::new(MetricsState {
                profile: initial_profile,
                configured_custom_bps: custom_bps,
                light_bps,
                peak_7d_bps,
                game_mode_active: false,
                game_profile: BandwidthProfile::Light,
                game_custom_bps: light_bps,
                ema_speed_bps: 0.0,
                sparkline_60s: spark,
                ticks_mod_4: 0,
            }),
        });

        if let Ok(listener) = listener_res {
            let bucket_bg = bucket;
            let auth_bg = expected_auth_header;
            tokio::spawn(async move {
                loop {
                    let Ok((client, peer)) = listener.accept().await else {
                        break;
                    };
                    if !peer.ip().is_loopback() {
                        continue;
                    }
                    let b = bucket_bg.clone();
                    let auth = auth_bg.clone();
                    tokio::spawn(async move {
                        let _ = handle_proxy_client(client, b, &auth).await;
                    });
                }
            });
        }

        limiter
    }

    pub fn proxy_url(&self) -> Option<String> {
        self.port
            .map(|p| format!("http://{}:{}@127.0.0.1:{p}", self.username, self.password))
    }

    pub fn port(&self) -> Option<u16> {
        self.port
    }

    pub fn is_fallback_mode(&self) -> bool {
        self.port.is_none()
    }

    pub fn current_limit_bps(&self) -> u64 {
        self.bucket.rate_bps()
    }

    pub fn apply_profile(&self, profile: BandwidthProfile, custom_bps: Option<u64>, light_bps: Option<u64>) {
        let eff = {
            let mut m = self.metrics.lock();
            m.profile = profile;
            if let Some(c) = custom_bps {
                m.configured_custom_bps = c;
            }
            if let Some(l) = light_bps {
                m.light_bps = l;
            }
            if m.game_mode_active {
                compute_effective_limit(m.game_profile, m.game_custom_bps, m.light_bps, m.peak_7d_bps)
            } else {
                compute_effective_limit(m.profile, m.configured_custom_bps, m.light_bps, m.peak_7d_bps)
            }
        };
        self.bucket.set_rate_bps(eff);
    }

    pub fn set_game_mode_active(
        &self,
        active: bool,
        game_profile: BandwidthProfile,
        game_custom_bps: u64,
    ) {
        let eff = {
            let mut m = self.metrics.lock();
            m.game_mode_active = active;
            m.game_profile = game_profile;
            m.game_custom_bps = game_custom_bps;
            if active {
                compute_effective_limit(game_profile, game_custom_bps, m.light_bps, m.peak_7d_bps)
            } else {
                compute_effective_limit(m.profile, m.configured_custom_bps, m.light_bps, m.peak_7d_bps)
            }
        };
        self.bucket.set_rate_bps(eff);
    }

    /// Called every 250ms (4 Hz) to sample bandwidth, update 60s sparkline & 7-day peak, and return stats.
    pub fn sample_tick_250ms(&self) -> (LimiterStatsPayload, Option<u64>) {
        let bytes = self.bucket.drain_window_bytes();
        let inst_bps = (bytes * 4) as f64;
        let mut new_peak = None;

        let mut m = self.metrics.lock();
        if m.ema_speed_bps < 1.0 && inst_bps > 0.0 {
            m.ema_speed_bps = inst_bps;
        } else {
            m.ema_speed_bps = m.ema_speed_bps * 0.65 + inst_bps * 0.35;
        }
        let speed_bps = if m.ema_speed_bps < 512.0 && bytes == 0 {
            m.ema_speed_bps = 0.0;
            0
        } else {
            m.ema_speed_bps as u64
        };

        if speed_bps > m.peak_7d_bps && speed_bps > 256 * 1024 {
            m.peak_7d_bps = speed_bps;
            new_peak = Some(speed_bps);
        }

        m.ticks_mod_4 = (m.ticks_mod_4 + 1) % 4;
        if m.ticks_mod_4 == 0 {
            if m.sparkline_60s.len() >= 60 {
                m.sparkline_60s.pop_front();
            }
            m.sparkline_60s.push_back(speed_bps);
        }

        let active_profile = if m.game_mode_active {
            m.game_profile
        } else {
            m.profile
        };

        let payload = LimiterStatsPayload {
            speed_bps,
            limit_bps: self.bucket.rate_bps(),
            profile: active_profile,
            peak_7d_bps: m.peak_7d_bps,
            sparkline_60s: m.sparkline_60s.iter().copied().collect(),
            proxy_port: self.port,
            fallback_mode: self.port.is_none(),
            game_mode_override_active: m.game_mode_active,
        };
        (payload, new_peak)
    }
}

fn compute_effective_limit(
    profile: BandwidthProfile,
    custom_bps: u64,
    light_bps: u64,
    peak_7d_bps: u64,
) -> u64 {
    match profile {
        BandwidthProfile::Unlimited => 0,
        BandwidthProfile::Balanced => {
            let base = if peak_7d_bps > 1024 * 1024 {
                peak_7d_bps
            } else {
                10 * 1024 * 1024 // sensible 10 MB/s baseline until real peak is measured
            };
            ((base as f64) * 0.60) as u64
        }
        BandwidthProfile::Light => light_bps.max(128 * 1024),
        BandwidthProfile::Custom => custom_bps.max(128 * 1024),
    }
}

/// Opens a TCP connection to `host:port` with `SO_RCVBUF = 256 KB` set BEFORE `connect`
/// so Windows TCP receive window auto-tuning cannot buffer megabytes of burst traffic (Section 11.2).
async fn connect_tuned_upstream(host: &str, port: u16) -> io::Result<TcpStream> {
    let host_owned = host.to_string();
    let addrs: Vec<SocketAddr> = tokio::task::spawn_blocking(move || {
        (host_owned.as_str(), port)
            .to_socket_addrs()
            .map(|it| it.collect::<Vec<_>>())
    })
    .await
    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))??;

    let mut last_err = io::Error::new(io::ErrorKind::NotFound, "Nenhum endereço resolvido");
    for addr in addrs {
        let domain = if addr.is_ipv4() {
            Domain::IPV4
        } else {
            Domain::IPV6
        };
        let sock = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
        let _ = sock.set_recv_buffer_size(OUTBOUND_SO_RCVBUF);
        let _ = sock.set_nodelay(true);
        sock.set_nonblocking(true)?;

        match sock.connect(&addr.into()) {
            Ok(()) => {}
            Err(err)
                if err.raw_os_error() == Some(10035)
                    || err.raw_os_error() == Some(115)
                    || err.kind() == io::ErrorKind::WouldBlock =>
            {
                // WSAEWOULDBLOCK (10035) is expected for non-blocking connect on Windows
            }
            Err(err) => {
                last_err = err;
                continue;
            }
        }

        let std_stream: std::net::TcpStream = sock.into();
        let tokio_stream = TcpStream::from_std(std_stream)?;
        tokio_stream.writable().await?;
        if let Some(err) = tokio_stream.take_error()? {
            last_err = err;
            continue;
        }
        return Ok(tokio_stream);
    }
    Err(last_err)
}

async fn handle_proxy_client(
    mut client: TcpStream,
    bucket: Arc<TokenBucket>,
    expected_auth: &str,
) -> io::Result<()> {
    let _ = client.set_nodelay(true);
    let mut header_buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 1024];

    let header_end = loop {
        if header_buf.len() > 32 * 1024 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Cabeçalho HTTP muito longo"));
        }
        let n = client.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        header_buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find_double_crlf(&header_buf) {
            break pos;
        }
    };

    let header_str = String::from_utf8_lossy(&header_buf[..header_end]);
    let mut lines = header_str.lines();
    let req_line = lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Requisição vazia"))?;

    // Constant-time check of Proxy-Authorization
    let mut authed = false;
    for line in lines.clone() {
        if let Some((name, val)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("Proxy-Authorization") {
                if constant_time_eq(val.trim().as_bytes(), expected_auth.as_bytes()) {
                    authed = true;
                }
            }
        }
    }
    if !authed {
        client
            .write_all(
                b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"Boladao\"\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            )
            .await?;
        return Ok(());
    }

    let mut req_parts = req_line.split_whitespace();
    let method = req_parts.next().unwrap_or("");
    let target = req_parts.next().unwrap_or("");
    let version = req_parts.next().unwrap_or("HTTP/1.1");

    let leftover = &header_buf[header_end + 4..];

    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = parse_host_port(target, 443)?;
        let mut upstream = connect_tuned_upstream(&host, port).await?;
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
        if !leftover.is_empty() {
            upstream.write_all(leftover).await?;
        }
        relay_throttled_bidirectional(client, upstream, bucket).await?;
    } else {
        // Absolute-URI HTTP request (`GET http://host:port/path HTTP/1.1`)
        let parsed = url::Url::parse(target)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Sem host"))?
            .to_string();
        let port = parsed.port_or_known_default().unwrap_or(80);
        let mut path_and_query = parsed.path().to_string();
        if path_and_query.is_empty() {
            path_and_query.push('/');
        }
        if let Some(q) = parsed.query() {
            path_and_query.push('?');
            path_and_query.push_str(q);
        }

        let mut upstream = connect_tuned_upstream(&host, port).await?;
        let mut forwarded = format!("{method} {path_and_query} {version}\r\n");
        for line in header_str.lines().skip(1) {
            if let Some((name, _)) = line.split_once(':') {
                let n_trim = name.trim();
                if n_trim.eq_ignore_ascii_case("Proxy-Authorization")
                    || n_trim.eq_ignore_ascii_case("Proxy-Connection")
                {
                    continue;
                }
            }
            forwarded.push_str(line);
            forwarded.push_str("\r\n");
        }
        forwarded.push_str("Connection: close\r\n\r\n");
        upstream.write_all(forwarded.as_bytes()).await?;
        if !leftover.is_empty() {
            upstream.write_all(leftover).await?;
        }
        relay_throttled_bidirectional(client, upstream, bucket).await?;
    }

    Ok(())
}

async fn relay_throttled_bidirectional(
    client: TcpStream,
    upstream: TcpStream,
    bucket: Arc<TokenBucket>,
) -> io::Result<()> {
    let (mut c_read, mut c_write) = client.into_split();
    let (mut u_read, mut u_write) = upstream.into_split();

    let c2u = async move {
        let mut buf = [0u8; 8 * 1024];
        loop {
            let n = c_read.read(&mut buf).await?;
            if n == 0 {
                let _ = u_write.shutdown().await;
                break;
            }
            u_write.write_all(&buf[..n]).await?;
        }
        Ok::<(), io::Error>(())
    };

    let u2c = async move {
        let mut buf = [0u8; CHUNK_BYTES];
        loop {
            let n = u_read.read(&mut buf).await?;
            if n == 0 {
                let _ = c_write.shutdown().await;
                break;
            }
            bucket.acquire(n).await;
            bucket.record_bytes(n);
            c_write.write_all(&buf[..n]).await?;
        }
        Ok::<(), io::Error>(())
    };

    let _ = tokio::join!(c2u, u2c);
    Ok(())
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_host_port(target: &str, default_port: u16) -> io::Result<(String, u16)> {
    if let Some((h, p)) = target.rsplit_once(':') {
        if let Ok(port) = p.parse::<u16>() {
            return Ok((h.trim_matches(['[', ']']).to_string(), port));
        }
    }
    Ok((target.trim_matches(['[', ']']).to_string(), default_port))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Starts a tiny local HTTP server that streams `total_bytes` of payload.
    async fn start_dummy_http_origin(total_bytes: usize) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut req_buf = [0u8; 1024];
                    let _ = sock.read(&mut req_buf).await;
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {total_bytes}\r\nConnection: close\r\n\r\n"
                    );
                    if sock.write_all(header.as_bytes()).await.is_err() {
                        return;
                    }
                    let chunk = vec![0xABu8; 32 * 1024];
                    let mut rem = total_bytes;
                    while rem > 0 {
                        let send = rem.min(chunk.len());
                        if sock.write_all(&chunk[..send]).await.is_err() {
                            break;
                        }
                        rem -= send;
                    }
                });
            }
        });
        port
    }

    #[tokio::test]
    async fn token_bucket_20mb_at_2mbps_finishes_in_10s_and_changes_rate_live() {
        let total_size = 20 * 1024 * 1024; // 20 MB
        let rate_2mbps = 2 * 1024 * 1024; // 2 MB/s
        let origin_port = start_dummy_http_origin(total_size).await;

        let limiter =
            BandwidthLimiter::start(BandwidthProfile::Custom, rate_2mbps, rate_2mbps, 0).await;
        let proxy_url = limiter.proxy_url().expect("Proxy local deve subir");

        let client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::http(&proxy_url).unwrap())
            .build()
            .unwrap();

        // Part A: 20 MB at 2 MB/s -> 10 s ± 10% (9.0 s .. 11.0 s)
        let t0 = Instant::now();
        let resp = client
            .get(format!("http://127.0.0.1:{origin_port}/video.bin"))
            .send()
            .await
            .unwrap();
        let body = resp.bytes().await.unwrap();
        let elapsed = t0.elapsed().as_secs_f64();
        assert_eq!(body.len(), total_size);
        assert!(
            (9.0..=11.0).contains(&elapsed),
            "20 MB a 2 MB/s deveria levar ~10s (±10%), levou {elapsed:.2}s"
        );

        // Part B: Dynamic rate change mid-transfer reacts in < 1s
        // Start a 16 MB download at 1 MB/s; after 2s (~2 MB transferred, 14 MB left),
        // switch to Unlimited (0). Remaining 14 MB finishes almost immediately (< 1s)!
        let limiter_dyn =
            BandwidthLimiter::start(BandwidthProfile::Custom, 1024 * 1024, 1024 * 1024, 0).await;
        let client_dyn = reqwest::Client::builder()
            .proxy(reqwest::Proxy::http(limiter_dyn.proxy_url().unwrap()).unwrap())
            .build()
            .unwrap();
        let origin_16mb = start_dummy_http_origin(16 * 1024 * 1024).await;

        let limiter_switch = limiter_dyn.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            limiter_switch.apply_profile(BandwidthProfile::Unlimited, None, None);
        });

        let t_dyn = Instant::now();
        let bytes_dyn = client_dyn
            .get(format!("http://127.0.0.1:{origin_16mb}/fast.bin"))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let dyn_elapsed = t_dyn.elapsed().as_secs_f64();
        assert_eq!(bytes_dyn.len(), 16 * 1024 * 1024);
        assert!(
            (2.0..2.95).contains(&dyn_elapsed),
            "Mudança de limite ao vivo deveria acelerar em <1s após t=2s, levou {dyn_elapsed:.2}s"
        );
    }
}
