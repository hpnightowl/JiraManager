//! Opening web pages. The window can't open arbitrary addresses; it asks here, and only the Jira
//! site, the GitLab servers and the Ollama download page are opened.

use crate::error::AppError;
use crate::jira::KEY_URL;
use crate::tasks::TaskStore;
use reqwest::Url;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

const OLLAMA_DOWNLOAD: &str = "https://ollama.com";

fn origin(url: &Url) -> String {
    url.origin().ascii_serialization()
}

/// `Ok(url)` when `link` is https (http only on this computer) and on one of the `allowed` sites.
pub fn check_link(link: &str, allowed: &[String]) -> Result<Url, AppError> {
    let refuse = || AppError::Invalid("That link isn't on a site you set up (Jira or GitLab), so it was not opened.".into());
    let url = Url::parse(link.trim()).map_err(|_| refuse())?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    let scheme_ok = url.scheme() == "https" || (url.scheme() == "http" && local);
    // A user name or password in the address is a classic way to make a bad link look good.
    if !scheme_ok || !url.username().is_empty() || url.password().is_some() {
        return Err(refuse());
    }
    let here = origin(&url);
    allowed.iter().any(|a| Url::parse(a).is_ok_and(|a| origin(&a) == here)).then_some(url).ok_or_else(refuse)
}

fn allowed_sites(store: &TaskStore) -> Result<Vec<String>, AppError> {
    let mut sites = vec![OLLAMA_DOWNLOAD.to_string()];
    sites.extend(store.get_setting(KEY_URL)?.filter(|u| !u.is_empty()));
    sites.extend(store.connections()?.into_iter().filter(|c| c.kind == "gitlab").map(|c| c.base_url));
    Ok(sites)
}

#[tauri::command]
pub fn open_link(url: String, app: AppHandle, store: State<'_, TaskStore>) -> Result<(), AppError> {
    let url = check_link(&url, &allowed_sites(&store)?)?;
    app.opener().open_url(url.as_str(), None::<&str>).map_err(|e| AppError::Invalid(format!("Couldn't open the link: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sites() -> Vec<String> {
        vec!["https://acme.atlassian.net".into(), "https://git.client.example".into(), "https://ollama.com".into()]
    }

    #[test]
    fn links_on_your_own_servers_are_allowed() {
        for ok in [
            "https://acme.atlassian.net/browse/ABC-1",
            "https://git.client.example/shop/web/-/merge_requests/4",
            "https://ollama.com/download",
        ] {
            assert!(check_link(ok, &sites()).is_ok(), "{ok}");
        }
    }

    #[test]
    fn everything_else_is_refused() {
        for bad in [
            "https://evil.example/phish",
            "https://acme.atlassian.net.evil.example/browse/ABC-1", // look-alike host
            "https://acme.atlassian.net@evil.example/x",            // the real host is evil.example
            "https://user:pw@acme.atlassian.net/x",                 // credentials in the address
            "http://acme.atlassian.net/browse/ABC-1",               // not encrypted
            "https://acme.atlassian.net:8443/x",                    // a different port is a different site
            "file:///etc/passwd",
            "javascript:alert(1)",
            "mailto:a@b.c",
            "not a url",
            "",
        ] {
            assert!(check_link(bad, &sites()).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_server_on_this_computer_may_use_plain_http() {
        let local = vec!["http://localhost:8080".to_string()];
        assert!(check_link("http://localhost:8080/browse/ABC-1", &local).is_ok());
        assert!(check_link("http://localhost:9999/browse/ABC-1", &local).is_err());
    }

    #[test]
    fn the_list_comes_from_your_settings_and_git_servers() {
        let store = TaskStore::in_memory();
        store.set_setting(KEY_URL, "https://acme.atlassian.net").unwrap();
        store.add_connection("Work", "gitlab", "https://gitlab.example", None, None).unwrap();
        store.add_connection("Repo", "local", "/home/me/repo", None, None).unwrap();
        let allowed = allowed_sites(&store).unwrap();
        assert!(allowed.contains(&"https://acme.atlassian.net".to_string()) && allowed.contains(&"https://gitlab.example".to_string()));
        assert!(!allowed.iter().any(|a| a.contains("/home/me")), "a local folder is not a website");
    }
}
