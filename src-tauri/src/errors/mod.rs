use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ErrorAction {
    None,
    Retry,
    UpdateYtdlp,
    Login { platform: String },
    ChangeFormat,
    ChangeDestination,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobError {
    pub code: String,
    pub message_pt: String,
    pub hint_pt: Option<String>,
    pub action: ErrorAction,
    pub transient: bool,
    pub suggest_impersonate: bool,
}

/// Maps `yt-dlp` or `ffmpeg` stderr text into clear Brazilian Portuguese messages and actionable recovery options (Section 9.6).
pub fn map_stderr_to_job_error(stderr: &str, platform_id: &str, drive_label: &str) -> JobError {
    let lower = stderr.to_ascii_lowercase();

    if lower.contains("sign in to confirm you’re not a bot")
        || lower.contains("sign in to confirm you're not a bot")
        || lower.contains("confirm you are not a bot")
    {
        return JobError {
            code: "bot_check".to_string(),
            message_pt: tr!(
                "O YouTube pediu login pra confirmar que você não é robô.",
                "YouTube asked for a sign-in to confirm you're not a bot.",
                "YouTube pidió iniciar sesión para confirmar que no eres un robot."
            ),
            hint_pt: Some(tr!(
                "Conecte uma conta secundária em Ajustes → Contas ou tente novamente em alguns minutos.",
                "Connect a secondary account in Settings → Accounts or try again in a few minutes.",
                "Conecta una cuenta secundaria en Ajustes → Cuentas o vuelve a intentarlo en unos minutos."
            )),
            action: ErrorAction::Login {
                platform: platform_id.to_string(),
            },
            transient: false,
            suggest_impersonate: false,
        };
    }

    if lower.contains("http error 429") || lower.contains("too many requests") {
        return JobError {
            code: "http_429".to_string(),
            message_pt: tr!(
                "Muitas requisições. O app vai esperar e tentar de novo.",
                "Too many requests. The app will wait and try again.",
                "Demasiadas solicitudes. La app esperará y volverá a intentarlo."
            ),
            hint_pt: Some(tr!(
                "Reduzir a concorrência em Ajustes → Rede ajuda a evitar bloqueios temporários.",
                "Lowering simultaneous downloads in Settings → Network helps avoid temporary blocks.",
                "Reducir las descargas simultáneas en Ajustes → Red ayuda a evitar bloqueos temporales."
            )),
            action: ErrorAction::Retry,
            transient: true,
            suggest_impersonate: false,
        };
    }

    if lower.contains("private video")
        || lower.contains("login required")
        || lower.contains("sign in to view")
        || lower.contains("members-only")
        || lower.contains("this video is available to this channel's members")
        || lower.contains("authentication")
        || lower.contains("age-restricted")
    {
        return JobError {
            code: "auth_required".to_string(),
            message_pt: tr!(
                "Esse conteúdo é privado, restrito por idade ou exige login.",
                "This content is private, age-restricted or requires a sign-in.",
                "Este contenido es privado, tiene restricción de edad o requiere iniciar sesión."
            ),
            hint_pt: Some(tr!(
                "Conecte sua conta desta plataforma em Ajustes → Contas.",
                "Connect your account for this platform in Settings → Accounts.",
                "Conecta tu cuenta de esta plataforma en Ajustes → Cuentas."
            )),
            action: ErrorAction::Login {
                platform: platform_id.to_string(),
            },
            transient: false,
            suggest_impersonate: false,
        };
    }

    if lower.contains("not available in your country")
        || lower.contains("geo-restricted")
        || lower.contains("blocked it in your country")
    {
        return JobError {
            code: "geo_blocked".to_string(),
            message_pt: tr!(
                "Bloqueado na sua região.",
                "Blocked in your region.",
                "Bloqueado en tu región."
            ),
            hint_pt: None,
            action: ErrorAction::None,
            transient: false,
            suggest_impersonate: false,
        };
    }

    if lower.contains("video unavailable")
        || lower.contains("has been removed")
        || lower.contains("account associated with this video has been terminated")
        || lower.contains("http error 404")
    {
        return JobError {
            code: "unavailable".to_string(),
            message_pt: tr!(
                "Vídeo indisponível ou removido.",
                "Video unavailable or removed.",
                "Video no disponible o eliminado."
            ),
            hint_pt: None,
            action: ErrorAction::None,
            transient: false,
            suggest_impersonate: false,
        };
    }

    if lower.contains("requested format is not available") {
        return JobError {
            code: "format_unavailable".to_string(),
            message_pt: tr!(
                "Esse formato não existe pra essa mídia.",
                "This format isn't available for this media.",
                "Este formato no está disponible para este contenido."
            ),
            hint_pt: Some(tr!(
                "Escolha outro formato ou resolução no seletor.",
                "Pick another format or resolution in the selector.",
                "Elige otro formato o resolución en el selector."
            )),
            action: ErrorAction::ChangeFormat,
            transient: false,
            suggest_impersonate: false,
        };
    }

    if lower.contains("no space left on device")
        || lower.contains("not enough space on the disk")
        || lower.contains("errno 28")
    {
        return JobError {
            code: "disk_full".to_string(),
            message_pt: tr!(
                "Sem espaço em {drive_label}.",
                "No space left on {drive_label}.",
                "Sin espacio en {drive_label}."
            ),
            hint_pt: Some(tr!(
                "Libere espaço ou escolha outra pasta de destino.",
                "Free up space or choose another destination folder.",
                "Libera espacio o elige otra carpeta de destino."
            )),
            action: ErrorAction::ChangeDestination,
            transient: false,
            suggest_impersonate: false,
        };
    }

    if lower.contains("unsupported url") {
        return JobError {
            code: "unsupported_url".to_string(),
            message_pt: tr!(
                "O yt-dlp não reconhece esse link.",
                "yt-dlp doesn't recognize this link.",
                "yt-dlp no reconoce este enlace."
            ),
            hint_pt: Some(tr!(
                "Se o site for suportado numa versão recente, atualize o yt-dlp.",
                "If a newer version supports the site, update yt-dlp.",
                "Si una versión reciente admite el sitio, actualiza yt-dlp."
            )),
            action: ErrorAction::UpdateYtdlp,
            transient: false,
            suggest_impersonate: false,
        };
    }

    if lower.contains("unable to extract")
        || lower.contains("nsig")
        || lower.contains("signature extraction failed")
        || lower.contains("js runtime")
        || lower.contains("javascript runtime")
        || lower.contains("extractor error")
    {
        return JobError {
            code: "extractor_broken".to_string(),
            message_pt: tr!(
                "O site mudou e o yt-dlp precisa de atualização.",
                "The site changed and yt-dlp needs an update.",
                "El sitio cambió y yt-dlp necesita una actualización."
            ),
            hint_pt: Some(tr!(
                "Atualize o yt-dlp com um clique e tente de novo.",
                "Update yt-dlp with one click and try again.",
                "Actualiza yt-dlp con un clic y vuelve a intentarlo."
            )),
            action: ErrorAction::UpdateYtdlp,
            transient: false,
            suggest_impersonate: false,
        };
    }

    // Cloudflare / anti-bot 403 -> triggers one silent `--impersonate` retry (Section 7.5)
    if lower.contains("cloudflare")
        || lower.contains("http error 403")
        || lower.contains("just a moment")
    {
        return JobError {
            code: "anti_bot_403".to_string(),
            message_pt: tr!(
                "O site bloqueou a conexão automática (erro 403).",
                "The site blocked the automated connection (error 403).",
                "El sitio bloqueó la conexión automática (error 403)."
            ),
            hint_pt: Some(tr!(
                "Tente de novo ou atualize o yt-dlp se o erro persistir.",
                "Try again, or update yt-dlp if the error persists.",
                "Vuelve a intentarlo o actualiza yt-dlp si el error continúa."
            )),
            action: ErrorAction::UpdateYtdlp,
            transient: true,
            suggest_impersonate: true,
        };
    }

    if lower.contains("timed out")
        || lower.contains("connection reset")
        || lower.contains("network is unreachable")
        || lower.contains("http error 50")
        || lower.contains("http error 52")
        || lower.contains("fragment")
        || lower.contains("eof occurred")
    {
        return JobError {
            code: "network_transient".to_string(),
            message_pt: tr!(
                "A conexão oscilou durante o download.",
                "The connection was unstable during the download.",
                "La conexión fue inestable durante la descarga."
            ),
            hint_pt: Some(tr!(
                "O Boladão retoma de onde parou automaticamente.",
                "Boladão picks up where it left off automatically.",
                "Boladão continúa automáticamente desde donde se quedó."
            )),
            action: ErrorAction::Retry,
            transient: true,
            suggest_impersonate: false,
        };
    }

    // Fallback: take the cleanest ERROR: line without raw stack traces
    let summary = stderr
        .lines()
        .rev()
        .find(|l| l.contains("ERROR:") || !l.trim().is_empty())
        .map(|l| l.trim().trim_start_matches("ERROR:").trim().to_string())
        .unwrap_or_else(|| {
            tr!(
                "Falha inesperada ao processar a mídia.",
                "Unexpected failure while processing the media.",
                "Error inesperado al procesar el contenido."
            )
        });

    JobError {
        code: "unknown".to_string(),
        message_pt: tr!(
            "Não deu pra concluir: {summary}",
            "Couldn't finish: {summary}",
            "No se pudo completar: {summary}"
        ),
        hint_pt: Some(tr!(
                "Abra o log do item para ver os detalhes técnicos.",
                "Open the item's log to see the technical details.",
                "Abre el registro del elemento para ver los detalles técnicos."
            )),
        action: ErrorAction::Retry,
        transient: false,
        suggest_impersonate: false,
    }
}
