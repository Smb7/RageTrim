use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::error::Error;

const SERVICE: &str = "RageTrim";
const ACCOUNT: &str = "discord-webhook";
const MAX_MESSAGE: usize = 2000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscordStatus {
    pub configured: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookCheck {
    pub name: String,
}

pub fn save_webhook(url: &str) -> Result<(), Error> {
    let url = normalize_webhook(url)?;
    let entry = keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|_| Error::message("Couldn't open Windows Credential Manager."))?;
    entry
        .set_password(&url)
        .map_err(|_| Error::message("Couldn't save the webhook in Windows Credential Manager."))?;
    Ok(())
}

pub fn clear_webhook() -> Result<(), Error> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|_| Error::message("Couldn't open Windows Credential Manager."))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(Error::message("Couldn't remove the saved webhook.")),
    }
}

pub fn status() -> Result<DiscordStatus, Error> {
    Ok(DiscordStatus {
        configured: load_webhook().is_ok(),
    })
}

pub fn test_webhook() -> Result<WebhookCheck, Error> {
    let url = load_webhook()?;
    let response = client()?
        .get(url)
        .send()
        .map_err(|_| Error::message("Couldn't reach Discord."))?;
    if !response.status().is_success() {
        return Err(Error::message("Discord rejected that webhook."));
    }
    let body: serde_json::Value = response
        .json()
        .map_err(|_| Error::message("Discord returned an unreadable webhook check."))?;
    let name = body
        .get("name")
        .and_then(|name| name.as_str())
        .unwrap_or("Discord webhook")
        .chars()
        .take(80)
        .collect();
    Ok(WebhookCheck { name })
}

pub fn post_file(path: &Path, file_name: &str, message: &str) -> Result<(), Error> {
    let url = load_webhook()?;
    let message = clean_message(message)?;
    let file_name = safe_file_name(file_name);
    let file = reqwest::blocking::multipart::Part::file(path)
        .map_err(|_| Error::message("Couldn't read the export."))?
        .file_name(file_name)
        .mime_str("video/mp4")
        .map_err(|_| Error::message("Couldn't prepare the upload."))?;
    let payload = serde_json::json!({ "content": message }).to_string();
    let form = reqwest::blocking::multipart::Form::new()
        .text("payload_json", payload)
        .part("files[0]", file);
    let response = client()?
        .post(url)
        .multipart(form)
        .send()
        .map_err(|_| Error::message("Couldn't reach Discord."))?;
    match response.status().as_u16() {
        200..=299 => Ok(()),
        413 => Err(Error::message(
            "Discord rejected the file as too large. Export a smaller compress and post that.",
        )),
        429 => Err(Error::message(rate_limit_message(response))),
        401 | 404 => Err(Error::message(
            "Discord rejected that webhook. Create a new one and save it again.",
        )),
        _ => Err(Error::message("Discord rejected the post.")),
    }
}

pub fn normalize_webhook(url: &str) -> Result<String, Error> {
    let url = url.trim();
    let parsed =
        reqwest::Url::parse(url).map_err(|_| Error::message("That webhook URL is invalid."))?;
    if parsed.scheme() != "https" {
        return Err(Error::message("Webhook URL must use https."));
    }
    let host = parsed.host_str().unwrap_or("");
    let allowed = matches!(
        host,
        "discord.com" | "discordapp.com" | "ptb.discord.com" | "canary.discord.com"
    );
    if !allowed || !parsed.path().starts_with("/api/webhooks/") {
        return Err(Error::message("Paste a Discord channel webhook URL."));
    }
    let parts: Vec<_> = parsed
        .path()
        .trim_end_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() != 4 || parts[2].is_empty() || parts[3].is_empty() {
        return Err(Error::message("That webhook URL is incomplete."));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(Error::message("That webhook URL is invalid."));
    }
    Ok(parsed.to_string())
}

pub fn clean_message(message: &str) -> Result<String, Error> {
    let message = message.trim();
    if message.chars().count() > MAX_MESSAGE {
        return Err(Error::message(
            "Discord messages must be 2000 characters or fewer.",
        ));
    }
    Ok(message.to_string())
}

fn load_webhook() -> Result<String, Error> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|_| Error::message("Couldn't open Windows Credential Manager."))?;
    let url = entry.get_password().map_err(|err| match err {
        keyring::Error::NoEntry => Error::message("Save a Discord webhook first."),
        _ => Error::message("Couldn't read the saved webhook."),
    })?;
    normalize_webhook(&url)
}

fn client() -> Result<reqwest::blocking::Client, Error> {
    reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(120))
        .user_agent("RageTrim")
        .build()
        .map_err(|_| Error::message("Couldn't start the Discord request."))
}

fn rate_limit_message(response: reqwest::blocking::Response) -> String {
    let retry = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(1.0);
    format!("Discord asked us to wait {retry:.0} seconds.")
}

fn safe_file_name(file_name: &str) -> String {
    let name = Path::new(file_name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("clip.mp4");
    let mut cleaned = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ' ') {
            cleaned.push(ch);
        } else {
            cleaned.push('_');
        }
    }
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "clip.mp4".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_discord_webhook_and_rejects_other_urls() {
        let ok =
            normalize_webhook("https://discord.com/api/webhooks/123456789012345678/secret-token")
                .unwrap();
        assert!(ok.starts_with("https://discord.com/api/webhooks/"));
        assert!(normalize_webhook("http://discord.com/api/webhooks/1/token").is_err());
        assert!(normalize_webhook("https://evil.example/api/webhooks/1/token").is_err());
        assert!(normalize_webhook("https://discord.com/api/v10/channels/1/messages").is_err());
        assert!(normalize_webhook("https://user:pass@discord.com/api/webhooks/1/token").is_err());
        assert!(clean_message("  hello  ").unwrap() == "hello");
        assert!(clean_message(&"x".repeat(2001)).is_err());
    }
}
