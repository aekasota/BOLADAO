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
            message_pt: "O YouTube pediu login pra confirmar que você não é robô.".to_string(),
            hint_pt: Some("Conecte uma conta secundária em Ajustes → Contas ou tente novamente em alguns minutos.".to_string()),
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
            message_pt: "Muitas requisições. O app vai esperar e tentar de novo.".to_string(),
            hint_pt: Some("Reduzir a concorrência em Ajustes → Rede ajuda a evitar bloqueios temporários.".to_string()),
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
            message_pt: "Esse conteúdo é privado, restrito por idade ou exige login.".to_string(),
            hint_pt: Some("Conecte sua conta desta plataforma em Ajustes → Contas.".to_string()),
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
            message_pt: "Bloqueado na sua região.".to_string(),
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
            message_pt: "Vídeo indisponível ou removido.".to_string(),
            hint_pt: None,
            action: ErrorAction::None,
            transient: false,
            suggest_impersonate: false,
        };
    }

    if lower.contains("requested format is not available") {
        return JobError {
            code: "format_unavailable".to_string(),
            message_pt: "Esse formato não existe pra essa mídia.".to_string(),
            hint_pt: Some("Escolha outro formato ou resolução no seletor.".to_string()),
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
            message_pt: format!("Sem espaço em {drive_label}."),
            hint_pt: Some("Libere espaço ou escolha outra pasta de destino.".to_string()),
            action: ErrorAction::ChangeDestination,
            transient: false,
            suggest_impersonate: false,
        };
    }

    if lower.contains("unsupported url") {
        return JobError {
            code: "unsupported_url".to_string(),
            message_pt: "O yt-dlp não reconhece esse link.".to_string(),
            hint_pt: Some("Se o site for suportado numa versão recente, atualize o yt-dlp.".to_string()),
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
            message_pt: "O site mudou e o yt-dlp precisa de atualização.".to_string(),
            hint_pt: Some("Atualize o yt-dlp com um clique e tente de novo.".to_string()),
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
            message_pt: "O site bloqueou a conexão automática (erro 403).".to_string(),
            hint_pt: Some("Tente de novo ou atualize o yt-dlp se o erro persistir.".to_string()),
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
            message_pt: "A conexão oscilou durante o download.".to_string(),
            hint_pt: Some("O Boladão retoma de onde parou automaticamente.".to_string()),
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
        .map(|l| l.trim().trim_start_matches("ERROR:").trim())
        .unwrap_or("Falha inesperada ao processar a mídia.");

    JobError {
        code: "unknown".to_string(),
        message_pt: format!("Não deu pra concluir: {summary}"),
        hint_pt: Some("Abra o log do item para ver os detalhes técnicos.".to_string()),
        action: ErrorAction::Retry,
        transient: false,
        suggest_impersonate: false,
    }
}
