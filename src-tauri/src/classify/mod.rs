use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaMode {
    Audio,
    Video,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Video,
    Short,
    Reel,
    Track,
    Album,
    Playlist,
    PlaylistItem,
    Channel,
    Live,
    Clip,
    Story,
    Post,
    Drm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Classification {
    pub normalized_url: String,
    pub dedup_key: String,
    pub platform_id: String,
    pub platform_name: String,
    pub brand_color: String,
    pub icon_slug: String,
    pub kind: SourceKind,
    pub default_mode: MediaMode,
    pub has_video_and_playlist: bool,
    pub requires_login_hint: bool,
    pub is_drm: bool,
    pub drm_message: Option<String>,
}

static URL_FINDER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"https?://[^\s<>"'`\)\]]+"#).unwrap());

/// Extracts all valid `http`/`https` URLs from arbitrary pasted text (e.g. a WhatsApp/Discord message).
pub fn extract_urls_from_text(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    for m in URL_FINDER_RE.find_iter(input) {
        let candidate = m
            .as_str()
            .trim_end_matches(|c: char| matches!(c, '.' | ',' | ';' | '!' | '?'));
        if let Ok(parsed) = Url::parse(candidate) {
            if matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some() {
                let s = parsed.to_string();
                if !out.contains(&s) {
                    out.push(s);
                }
            }
        }
    }
    out
}

/// Normalizes a URL for queue/library deduplication: strips tracking query params and canonicalizes `youtu.be`.
pub fn normalize_url_for_dedup(raw: &str) -> Option<String> {
    let parsed = Url::parse(raw.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    let clean_host = host.strip_prefix("www.").unwrap_or(&host);

    if clean_host == "youtu.be" {
        let vid = parsed.path().trim_matches('/');
        if !vid.is_empty() {
            return Some(format!("https://youtube.com/watch?v={vid}"));
        }
    }

    let mut clean = parsed.clone();
    let _ = clean.set_fragment(None);

    let tracking_keys = [
        "utm_source",
        "utm_medium",
        "utm_campaign",
        "utm_term",
        "utm_content",
        "si",
        "pp",
        "feature",
        "igshid",
        "igsh",
        "fbclid",
        "gclid",
        "s",
        "ref_src",
        "ref_url",
        "spm_id_from",
    ];

    let kept_pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| !tracking_keys.contains(&k.to_ascii_lowercase().as_str()))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    if kept_pairs.is_empty() {
        clean.set_query(None);
    } else {
        let mut qp = clean.query_pairs_mut();
        qp.clear();
        for (k, v) in kept_pairs {
            qp.append_pair(&k, &v);
        }
    }

    // Canonicalize mobile/www prefixes on YouTube for dedup
    let final_str = clean.to_string();
    Some(
        final_str
            .replace("://www.youtube.com/", "://youtube.com/")
            .replace("://m.youtube.com/", "://youtube.com/"),
    )
}

/// Pure, sub-millisecond local URL classification (<1 ms, zero network).
pub fn classify_url(raw: &str) -> Result<Classification, String> {
    let trimmed = raw.trim();
    let parsed = Url::parse(trimmed).map_err(|_| {
        tr!(
            "Link inválido. Use um endereço http:// ou https://.",
            "Invalid link. Use an http:// or https:// address.",
            "Enlace no válido. Usa una dirección http:// o https://."
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(tr!(
            "Apenas links http:// e https:// são aceitos.",
            "Only http:// and https:// links are accepted.",
            "Solo se aceptan enlaces http:// y https://."
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| tr!("Link sem domínio válido.", "The link has no valid domain.", "El enlace no tiene un dominio válido."))?
        .to_ascii_lowercase();
    let domain = host
        .strip_prefix("www.")
        .or_else(|| host.strip_prefix("m."))
        .unwrap_or(&host);

    let path = parsed.path();
    let path_lower = path.to_ascii_lowercase();
    let dedup_key = normalize_url_for_dedup(trimmed).unwrap_or_else(|| parsed.to_string());

    // 1. DRM check before anything else (Section 6)
    let is_drm_host = domain == "open.spotify.com"
        || domain == "spotify.com"
        || domain == "spotify.link"
        || domain.ends_with(".spotify.com")
        || domain == "netflix.com"
        || domain.ends_with(".netflix.com")
        || domain == "disneyplus.com"
        || domain.ends_with(".disneyplus.com")
        || domain == "primevideo.com"
        || domain.ends_with(".primevideo.com")
        || domain == "music.apple.com"
        || domain == "tv.apple.com"
        || domain == "deezer.com"
        || domain.ends_with(".deezer.com")
        || domain == "tidal.com"
        || domain.ends_with(".tidal.com");

    if is_drm_host {
        let (pid, pname, color, slug) = if domain.contains("spotify") {
            ("spotify", "Spotify", "#1DB954", "spotify")
        } else if domain.contains("netflix") {
            ("netflix", "Netflix", "#E50914", "netflix")
        } else if domain.contains("disney") {
            ("disneyplus", "Disney+", "#0063E5", "disney")
        } else if domain.contains("primevideo") {
            ("primevideo", "Prime Video", "#00A8E1", "primevideo")
        } else if domain.contains("apple") {
            ("applemusic", "Apple Music", "#FA243C", "applemusic")
        } else {
            ("drm", "Streaming DRM", "#FFB547", "globe")
        };

        return Ok(Classification {
            normalized_url: parsed.to_string(),
            dedup_key,
            platform_id: pid.to_string(),
            platform_name: pname.to_string(),
            brand_color: color.to_string(),
            icon_slug: slug.to_string(),
            kind: SourceKind::Drm,
            default_mode: MediaMode::Audio,
            has_video_and_playlist: false,
            requires_login_hint: false,
            is_drm: true,
            drm_message: Some(tr!(
                "Esse serviço usa DRM. O yt-dlp não baixa daqui.",
                "This service uses DRM. yt-dlp can't download from it.",
                "Este servicio usa DRM. yt-dlp no puede descargar de aquí."
            )),
        });
    }

    // 2. YouTube Music
    if host == "music.youtube.com" {
        let has_v = parsed.query_pairs().any(|(k, _)| k == "v");
        let has_list = parsed.query_pairs().any(|(k, _)| k == "list");
        let kind = if path_lower.starts_with("/playlist") || (has_list && !has_v) || path_lower.starts_with("/browse") {
            SourceKind::Playlist
        } else {
            SourceKind::Track
        };
        return Ok(Classification {
            normalized_url: parsed.to_string(),
            dedup_key,
            platform_id: "youtube_music".to_string(),
            platform_name: "YouTube Music".to_string(),
            brand_color: "#FF0033".to_string(),
            icon_slug: "youtubemusic".to_string(),
            kind,
            default_mode: MediaMode::Audio,
            has_video_and_playlist: has_v && has_list,
            requires_login_hint: false,
            is_drm: false,
            drm_message: None,
        });
    }

    // 3. YouTube
    if domain == "youtube.com" || domain == "youtu.be" || domain == "youtube-nocookie.com" {
        let has_v = domain == "youtu.be" && path.len() > 1
            || parsed.query_pairs().any(|(k, _)| k == "v");
        let has_list = parsed.query_pairs().any(|(k, _)| k == "list");

        let kind = if path_lower.starts_with("/shorts/") {
            SourceKind::Short
        } else if path_lower.starts_with("/live/") {
            SourceKind::Live
        } else if path_lower.starts_with("/clip/") {
            SourceKind::Clip
        } else if path_lower.starts_with("/playlist") || (has_list && !has_v) {
            SourceKind::Playlist
        } else if path_lower.starts_with("/@")
            || path_lower.starts_with("/channel/")
            || path_lower.starts_with("/c/")
            || path_lower.starts_with("/user/")
        {
            SourceKind::Channel
        } else {
            SourceKind::Video
        };

        return Ok(Classification {
            normalized_url: parsed.to_string(),
            dedup_key,
            platform_id: "youtube".to_string(),
            platform_name: "YouTube".to_string(),
            brand_color: "#FF0033".to_string(),
            icon_slug: "youtube".to_string(),
            kind,
            default_mode: MediaMode::Video,
            has_video_and_playlist: has_v && has_list,
            requires_login_hint: false,
            is_drm: false,
            drm_message: None,
        });
    }

    // 4. Instagram
    if domain == "instagram.com" || domain == "instagr.am" {
        let kind = if path_lower.starts_with("/reel/") || path_lower.starts_with("/reels/") {
            SourceKind::Reel
        } else if path_lower.starts_with("/stories/") {
            SourceKind::Story
        } else {
            SourceKind::Post
        };
        return Ok(Classification {
            normalized_url: parsed.to_string(),
            dedup_key,
            platform_id: "instagram".to_string(),
            platform_name: "Instagram".to_string(),
            brand_color: "#E4405F".to_string(),
            icon_slug: "instagram".to_string(),
            kind,
            default_mode: MediaMode::Video,
            has_video_and_playlist: false,
            requires_login_hint: matches!(kind, SourceKind::Story),
            is_drm: false,
            drm_message: None,
        });
    }

    // 5. TikTok
    if domain == "tiktok.com" || domain.ends_with(".tiktok.com") {
        let kind = if path_lower.contains("/video/") || path_lower.contains("/photo/") || host.starts_with("vm.") || host.starts_with("vt.") {
            SourceKind::Video
        } else if path_lower.starts_with("/@") {
            SourceKind::Channel
        } else {
            SourceKind::Video
        };
        return Ok(Classification {
            normalized_url: parsed.to_string(),
            dedup_key,
            platform_id: "tiktok".to_string(),
            platform_name: "TikTok".to_string(),
            brand_color: "#00F2EA".to_string(),
            icon_slug: "tiktok".to_string(),
            kind,
            default_mode: MediaMode::Video,
            has_video_and_playlist: false,
            requires_login_hint: false,
            is_drm: false,
            drm_message: None,
        });
    }

    // 6. X / Twitter
    if domain == "x.com" || domain == "twitter.com" || domain == "vxtwitter.com" || domain == "fxtwitter.com" {
        return Ok(Classification {
            normalized_url: parsed.to_string(),
            dedup_key,
            platform_id: "twitter".to_string(),
            platform_name: "X".to_string(),
            brand_color: "#F2EEE6".to_string(),
            icon_slug: "x".to_string(),
            kind: SourceKind::Post,
            default_mode: MediaMode::Video,
            has_video_and_playlist: false,
            requires_login_hint: false,
            is_drm: false,
            drm_message: None,
        });
    }

    // 7. Reddit
    if domain == "reddit.com" || domain.ends_with(".reddit.com") || domain == "redd.it" || domain == "v.redd.it" {
        return Ok(Classification {
            normalized_url: parsed.to_string(),
            dedup_key,
            platform_id: "reddit".to_string(),
            platform_name: "Reddit".to_string(),
            brand_color: "#FF4500".to_string(),
            icon_slug: "reddit".to_string(),
            kind: SourceKind::Post,
            default_mode: MediaMode::Video,
            has_video_and_playlist: false,
            requires_login_hint: false,
            is_drm: false,
            drm_message: None,
        });
    }

    // 8. Twitch
    if domain == "twitch.tv" || domain.ends_with(".twitch.tv") {
        let kind = if host == "clips.twitch.tv" || path_lower.contains("/clip/") {
            SourceKind::Clip
        } else if path_lower.starts_with("/videos/") {
            SourceKind::Video
        } else {
            SourceKind::Live
        };
        return Ok(Classification {
            normalized_url: parsed.to_string(),
            dedup_key,
            platform_id: "twitch".to_string(),
            platform_name: "Twitch".to_string(),
            brand_color: "#9146FF".to_string(),
            icon_slug: "twitch".to_string(),
            kind,
            default_mode: MediaMode::Video,
            has_video_and_playlist: false,
            requires_login_hint: false,
            is_drm: false,
            drm_message: None,
        });
    }

    // 9. Vimeo
    if domain == "vimeo.com" || domain.ends_with(".vimeo.com") {
        return Ok(Classification {
            normalized_url: parsed.to_string(),
            dedup_key,
            platform_id: "vimeo".to_string(),
            platform_name: "Vimeo".to_string(),
            brand_color: "#1AB7EA".to_string(),
            icon_slug: "vimeo".to_string(),
            kind: SourceKind::Video,
            default_mode: MediaMode::Video,
            has_video_and_playlist: false,
            requires_login_hint: false,
            is_drm: false,
            drm_message: None,
        });
    }

    // 10. SoundCloud
    if domain == "soundcloud.com" || domain.ends_with(".soundcloud.com") || domain == "on.soundcloud.com" {
        let kind = if path_lower.contains("/sets/") {
            SourceKind::Playlist
        } else {
            SourceKind::Track
        };
        return Ok(Classification {
            normalized_url: parsed.to_string(),
            dedup_key,
            platform_id: "soundcloud".to_string(),
            platform_name: "SoundCloud".to_string(),
            brand_color: "#FF5500".to_string(),
            icon_slug: "soundcloud".to_string(),
            kind,
            default_mode: MediaMode::Audio,
            has_video_and_playlist: false,
            requires_login_hint: false,
            is_drm: false,
            drm_message: None,
        });
    }

    // 11. Bandcamp
    if domain == "bandcamp.com" || domain.ends_with(".bandcamp.com") {
        let kind = if path_lower.starts_with("/album/") {
            SourceKind::Album
        } else {
            SourceKind::Track
        };
        return Ok(Classification {
            normalized_url: parsed.to_string(),
            dedup_key,
            platform_id: "bandcamp".to_string(),
            platform_name: "Bandcamp".to_string(),
            brand_color: "#629AA9".to_string(),
            icon_slug: "bandcamp".to_string(),
            kind,
            default_mode: MediaMode::Audio,
            has_video_and_playlist: false,
            requires_login_hint: false,
            is_drm: false,
            drm_message: None,
        });
    }

    // 12. Facebook
    if domain == "facebook.com" || domain.ends_with(".facebook.com") || domain == "fb.watch" {
        let kind = if path_lower.contains("/reel/") {
            SourceKind::Reel
        } else {
            SourceKind::Video
        };
        return Ok(Classification {
            normalized_url: parsed.to_string(),
            dedup_key,
            platform_id: "facebook".to_string(),
            platform_name: "Facebook".to_string(),
            brand_color: "#0866FF".to_string(),
            icon_slug: "facebook".to_string(),
            kind,
            default_mode: MediaMode::Video,
            has_video_and_playlist: false,
            requires_login_hint: false,
            is_drm: false,
            drm_message: None,
        });
    }

    // 13. Kick, Dailymotion, Bilibili
    if domain == "kick.com" {
        return Ok(Classification {
            normalized_url: parsed.to_string(),
            dedup_key,
            platform_id: "kick".to_string(),
            platform_name: "Kick".to_string(),
            brand_color: "#53FC18".to_string(),
            icon_slug: "kick".to_string(),
            kind: SourceKind::Video,
            default_mode: MediaMode::Video,
            has_video_and_playlist: false,
            requires_login_hint: false,
            is_drm: false,
            drm_message: None,
        });
    }
    if domain == "dailymotion.com" || domain == "dai.ly" {
        return Ok(Classification {
            normalized_url: parsed.to_string(),
            dedup_key,
            platform_id: "dailymotion".to_string(),
            platform_name: "Dailymotion".to_string(),
            brand_color: "#00D2F3".to_string(),
            icon_slug: "dailymotion".to_string(),
            kind: SourceKind::Video,
            default_mode: MediaMode::Video,
            has_video_and_playlist: false,
            requires_login_hint: false,
            is_drm: false,
            drm_message: None,
        });
    }
    if domain == "bilibili.com" || domain.ends_with(".bilibili.com") || domain == "b23.tv" {
        return Ok(Classification {
            normalized_url: parsed.to_string(),
            dedup_key,
            platform_id: "bilibili".to_string(),
            platform_name: "Bilibili".to_string(),
            brand_color: "#00A1D6".to_string(),
            icon_slug: "bilibili".to_string(),
            kind: SourceKind::Video,
            default_mode: MediaMode::Video,
            has_video_and_playlist: false,
            requires_login_hint: false,
            is_drm: false,
            drm_message: None,
        });
    }

    // 14. Generic / unknown domain — confirmed by yt-dlp extractor_key after probe
    Ok(Classification {
        normalized_url: parsed.to_string(),
        dedup_key,
        platform_id: "generic".to_string(),
        platform_name: domain.to_string(),
        brand_color: "#A8A39C".to_string(),
        icon_slug: "globe".to_string(),
        kind: SourceKind::Video,
        default_mode: MediaMode::Video,
        has_video_and_playlist: false,
        requires_login_hint: false,
        is_drm: false,
        drm_message: None,
    })
}

/// Refines a local `Classification` using the real `extractor_key` returned by `yt-dlp -J`.
pub fn refine_with_extractor_key(mut cls: Classification, extractor_key: &str, entry_count: Option<usize>) -> Classification {
    let ek = extractor_key.trim();
    if ek.is_empty() {
        return cls;
    }
    let lower = ek.to_ascii_lowercase();
    if cls.platform_id == "generic" {
        if lower.contains("youtube") {
            cls.platform_id = "youtube".to_string();
            cls.platform_name = "YouTube".to_string();
            cls.brand_color = "#FF0033".to_string();
            cls.icon_slug = "youtube".to_string();
        } else if lower.contains("instagram") {
            cls.platform_id = "instagram".to_string();
            cls.platform_name = "Instagram".to_string();
            cls.brand_color = "#E4405F".to_string();
            cls.icon_slug = "instagram".to_string();
        } else if lower.contains("soundcloud") {
            cls.platform_id = "soundcloud".to_string();
            cls.platform_name = "SoundCloud".to_string();
            cls.brand_color = "#FF5500".to_string();
            cls.icon_slug = "soundcloud".to_string();
            cls.default_mode = MediaMode::Audio;
        } else {
            cls.platform_name = ek.to_string();
        }
    }
    if entry_count.unwrap_or(0) > 1 && !matches!(cls.kind, SourceKind::Playlist | SourceKind::Album | SourceKind::Channel) {
        cls.kind = SourceKind::Playlist;
    }
    cls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_youtube_variants_and_music() {
        let short = classify_url("https://www.youtube.com/shorts/abc123XYZ").unwrap();
        assert_eq!(short.platform_id, "youtube");
        assert_eq!(short.kind, SourceKind::Short);
        assert_eq!(short.default_mode, MediaMode::Video);

        let youtu_be = classify_url("https://youtu.be/dQw4w9WgXcQ?si=tracking123").unwrap();
        assert_eq!(youtu_be.platform_id, "youtube");
        assert_eq!(youtu_be.kind, SourceKind::Video);
        assert_eq!(youtu_be.dedup_key, "https://youtube.com/watch?v=dQw4w9WgXcQ");

        let watch_list =
            classify_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL123456").unwrap();
        assert!(watch_list.has_video_and_playlist);
        assert_eq!(watch_list.kind, SourceKind::Video);

        let ytm = classify_url("https://music.youtube.com/watch?v=abc987").unwrap();
        assert_eq!(ytm.platform_id, "youtube_music");
        assert_eq!(ytm.kind, SourceKind::Track);
        assert_eq!(ytm.default_mode, MediaMode::Audio);
    }

    #[test]
    fn classifies_social_audio_and_unknown_domains() {
        let ig_reel = classify_url("https://www.instagram.com/reel/C9xyz123/?igsh=abc").unwrap();
        assert_eq!(ig_reel.platform_id, "instagram");
        assert_eq!(ig_reel.kind, SourceKind::Reel);

        let ig_post = classify_url("https://instagram.com/p/C8post/").unwrap();
        assert_eq!(ig_post.platform_id, "instagram");
        assert_eq!(ig_post.kind, SourceKind::Post);

        let ig_story = classify_url("https://instagram.com/stories/user/12345/").unwrap();
        assert!(ig_story.requires_login_hint);

        let sc_set = classify_url("https://soundcloud.com/artist/sets/ep-2026").unwrap();
        assert_eq!(sc_set.platform_id, "soundcloud");
        assert_eq!(sc_set.kind, SourceKind::Playlist);
        assert_eq!(sc_set.default_mode, MediaMode::Audio);

        let bc = classify_url("https://buryme.bandcamp.com/album/ost").unwrap();
        assert_eq!(bc.platform_id, "bandcamp");
        assert_eq!(bc.kind, SourceKind::Album);
        assert_eq!(bc.default_mode, MediaMode::Audio);

        let unknown = classify_url("https://arquivos.exemplo.org/aula01.mp4").unwrap();
        assert_eq!(unknown.platform_id, "generic");
        assert_eq!(unknown.icon_slug, "globe");
        assert!(!unknown.is_drm);
    }

    #[test]
    fn blocks_drm_platforms_immediately() {
        for url in [
            "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
            "https://www.netflix.com/watch/80100172",
            "https://music.apple.com/br/album/12345",
            "https://www.disneyplus.com/video/abc",
            "https://www.primevideo.com/detail/xyz",
        ] {
            let c = classify_url(url).unwrap();
            assert!(c.is_drm, "Expected DRM for {url}");
            assert_eq!(
                c.drm_message.as_deref(),
                Some("Esse serviço usa DRM. O yt-dlp não baixa daqui.")
            );
        }
    }

    #[test]
    fn extracts_multiple_urls_from_chat_paste() {
        let msg = "Olha esses 3 links:\n1) https://youtu.be/aaa111!\n2) https://www.instagram.com/reel/bbb222,\n3) https://x.com/user/status/333.";
        let urls = extract_urls_from_text(msg);
        assert_eq!(
            urls,
            vec![
                "https://youtu.be/aaa111",
                "https://www.instagram.com/reel/bbb222",
                "https://x.com/user/status/333",
            ]
        );
    }
}
