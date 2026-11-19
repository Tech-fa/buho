use serde_json::{Value, json};
use url::Url;

const MAX_BODY: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

pub fn origins_from_instance(app_url: Option<&str>, api_url: Option<&str>) -> Vec<String> {
    let mut origins = vec![
        "http://127.0.0.1".into(),
        "http://localhost".into(),
        "https://127.0.0.1".into(),
        "https://localhost".into(),
        "http://[::1]".into(),
        "https://[::1]".into(),
    ];
    for value in [app_url, api_url].into_iter().flatten() {
        if let Ok(url) = Url::parse(value) {
            origins.push(origin_of(&url));
        }
    }
    origins.sort();
    origins.dedup();
    origins
}

pub fn origin_of(url: &Url) -> String {
    let scheme = url.scheme();
    let host = url.host_str().unwrap_or("");
    match url.port() {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    }
}

pub fn is_localhost(url: &Url) -> bool {
    matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"))
}

pub fn allowed(url: &Url, origins: &[String]) -> bool {
    if url.scheme() != "http" && url.scheme() != "https" {
        return false;
    }
    if is_localhost(url) {
        return true;
    }
    let origin = origin_of(url);
    origins.iter().any(|item| item == &origin)
}

pub fn redact_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            if name.eq_ignore_ascii_case("authorization") {
                (name.clone(), "redacted".into())
            } else {
                (name.clone(), value.clone())
            }
        })
        .collect()
}

pub fn truncate_body(body: &str) -> (String, bool) {
    if body.len() <= MAX_BODY {
        (body.to_string(), false)
    } else {
        (body[..MAX_BODY].to_string(), true)
    }
}

pub fn parse_curl(command: &str) -> Result<HttpRequest, String> {
    let tokens = tokenize(command)?;
    if tokens.is_empty() {
        return Err("Paste a curl command.".into());
    }
    let mut index = 0;
    if tokens[0] == "curl" {
        index = 1;
    }
    let mut method = String::new();
    let mut url = String::new();
    let mut headers = Vec::new();
    let mut body = None;
    while index < tokens.len() {
        let token = &tokens[index];
        match token.as_str() {
            "-X" | "--request" => {
                index += 1;
                method = tokens
                    .get(index)
                    .cloned()
                    .ok_or_else(|| "curl is missing the request method.".to_string())?;
            }
            "-H" | "--header" => {
                index += 1;
                let raw = tokens
                    .get(index)
                    .cloned()
                    .ok_or_else(|| "curl is missing a header.".to_string())?;
                let (name, value) = raw.split_once(':').ok_or_else(|| {
                    "Each curl header needs a name and a value, like Content-Type: application/json."
                        .to_string()
                })?;
                let name = name.trim();
                let value = value.trim();
                if name.is_empty()
                    || name.contains(['\r', '\n', ':'])
                    || value.contains(['\r', '\n'])
                {
                    return Err("That curl header is not allowed.".into());
                }
                headers.push((name.to_string(), value.to_string()));
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" => {
                index += 1;
                body = Some(
                    tokens
                        .get(index)
                        .cloned()
                        .ok_or_else(|| "curl is missing the request body.".to_string())?,
                );
                if method.is_empty() {
                    method = "POST".into();
                }
            }
            "--url" => {
                index += 1;
                url = tokens
                    .get(index)
                    .cloned()
                    .ok_or_else(|| "curl is missing the URL.".to_string())?;
            }
            "-o" | "--output" | "-w" | "--write-out" | "-A" | "--user-agent" => {
                index += 1;
            }
            "-s" | "--silent" | "-S" | "--show-error" | "-L" | "--location" | "-i"
            | "--include" | "-I" | "--head" | "-k" | "--insecure" | "-v" | "--verbose" | "-g"
            | "--globoff" | "--compressed" => {}
            other if other.starts_with('-') => {}
            other => {
                if url.is_empty() {
                    url = other.to_string();
                }
            }
        }
        index += 1;
    }
    if url.is_empty() {
        return Err("curl is missing a URL.".into());
    }
    if method.is_empty() {
        method = if body.is_some() {
            "POST".into()
        } else {
            "GET".into()
        };
    }
    let method = method.to_uppercase();
    if !valid_method(&method) {
        return Err("That HTTP method is not allowed.".into());
    }
    Ok(HttpRequest {
        method,
        url,
        headers,
        body,
    })
}

fn valid_method(method: &str) -> bool {
    matches!(
        method,
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
    )
}

fn tokenize(command: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(ch) = chars.next() {
        if let Some(active) = quote {
            if ch == '\\' && active == '"' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
                continue;
            }
            if ch == active {
                quote = None;
                continue;
            }
            current.push(ch);
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            c if c.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if quote.is_some() {
        return Err("The curl command has an unfinished quote.".into());
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

pub fn request(req: &HttpRequest, origins: &[String]) -> Result<Value, String> {
    if !valid_method(&req.method) {
        return Err("That HTTP method is not allowed.".into());
    }
    let mut current = Url::parse(&req.url).map_err(|_| "The URL is not valid.".to_string())?;
    if current.scheme() == "file" {
        return Err("file: URLs are not allowed.".into());
    }
    if !allowed(&current, origins) {
        return Err("That address is not on the allowlist.".into());
    }
    let mut hops = 0;
    let mut method = req.method.clone();
    let mut body = req.body.clone();
    loop {
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout(std::time::Duration::from_secs(20))
            .build();
        let mut builder = agent.request(&method, current.as_str());
        for (name, value) in &req.headers {
            if name.contains(['\r', '\n', ':']) || value.contains(['\r', '\n']) {
                return Err("That header is not allowed.".into());
            }
            builder = builder.set(name, value);
        }
        let result = match &body {
            Some(payload) if method != "GET" && method != "HEAD" => builder.send_string(payload),
            _ => builder.call(),
        };
        match result {
            Ok(response) => return read_response(response),
            Err(ureq::Error::Status(code, response)) if (300..400).contains(&code) => {
                let location = response
                    .header("location")
                    .ok_or_else(|| "The server redirected without a location.".to_string())?;
                current = current
                    .join(location)
                    .map_err(|_| "The redirect location is not valid.".to_string())?;
                if current.scheme() == "file" || !allowed(&current, origins) {
                    return Err("The redirect left the allowlist.".into());
                }
                if matches!(code, 301 | 302 | 303) {
                    method = "GET".into();
                    body = None;
                }
                hops += 1;
                if hops > 8 {
                    return Err("Too many redirects.".into());
                }
            }
            Err(ureq::Error::Status(code, response)) => {
                let mut payload = read_response(response)?;
                payload["status"] = json!(code);
                payload["ok"] = json!(false);
                return Ok(payload);
            }
            Err(err) => return Err(format!("The request failed: {err}")),
        }
    }
}

fn read_response(response: ureq::Response) -> Result<Value, String> {
    let status = response.status();
    let headers: Vec<(String, String)> = response
        .headers_names()
        .into_iter()
        .filter_map(|name| {
            response
                .header(&name)
                .map(|value| (name, value.to_string()))
        })
        .collect();
    let raw = response.into_string().unwrap_or_default();
    let (body, truncated) = truncate_body(&raw);
    Ok(json!({
        "ok": status < 400,
        "status": status,
        "headers": redact_headers(&headers),
        "body": body,
        "truncated": truncated
    }))
}

pub fn request_json(arguments: &Value, origins: &[String]) -> Result<Value, String> {
    let method = arguments
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .trim()
        .to_uppercase();
    if !valid_method(&method) {
        return Err("That HTTP method is not allowed.".into());
    }
    let url = arguments
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| "url is required.".to_string())?;
    let mut headers = Vec::new();
    if let Some(object) = arguments.get("headers").and_then(Value::as_object) {
        for (name, value) in object {
            let value = value.as_str().unwrap_or(&value.to_string()).to_string();
            if name.contains(['\r', '\n', ':']) || value.contains(['\r', '\n']) {
                return Err("That header is not allowed.".into());
            }
            headers.push((name.clone(), value));
        }
    }
    let body = arguments.get("body").map(|value| match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    });
    request(
        &HttpRequest {
            method,
            url: url.to_string(),
            headers,
            body,
        },
        origins,
    )
}

pub fn curl_json(arguments: &Value, origins: &[String]) -> Result<Value, String> {
    let command = arguments
        .get("command")
        .and_then(Value::as_str)
        .or_else(|| arguments.get("curl").and_then(Value::as_str))
        .ok_or_else(|| "command is required.".to_string())?;
    let parsed = parse_curl(command)?;
    request(&parsed, origins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_post_with_headers() {
        let req = parse_curl(
            r#"curl 'https://api.example.com/items' -X POST -H 'Content-Type: application/json' -H 'Authorization: Bearer secret' --data '{"ok":true}'"#,
        )
        .unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.url, "https://api.example.com/items");
        assert_eq!(
            req.headers
                .iter()
                .find(|(name, _)| name == "Authorization")
                .map(|(_, value)| value.as_str()),
            Some("Bearer secret")
        );
        assert_eq!(req.body.as_deref(), Some(r#"{"ok":true}"#));
    }

    #[test]
    fn localhost_is_allowed_and_file_is_not() {
        let origins = origins_from_instance(
            Some("https://app.example.com"),
            Some("https://api.example.com"),
        );
        let local = Url::parse("http://127.0.0.1:3012/health").unwrap();
        let app = Url::parse("https://app.example.com/home").unwrap();
        let other = Url::parse("https://evil.example").unwrap();
        let file = Url::parse("file:///etc/passwd").unwrap();
        assert!(allowed(&local, &origins));
        assert!(allowed(&app, &origins));
        assert!(!allowed(&other, &origins));
        assert!(!allowed(&file, &origins));
    }

    #[test]
    fn redacts_authorization() {
        let redacted = redact_headers(&[("Authorization".into(), "Bearer secret".into())]);
        assert_eq!(redacted[0].1, "redacted");
    }

    #[test]
    fn rejects_header_injection_and_unknown_methods() {
        assert!(parse_curl("curl https://example.com -H 'X-A: one\r\nX-B: two'").is_err());
        assert!(parse_curl("curl https://example.com -X TRACE").is_err());
        assert!(parse_curl("curl 'https://example.com'").unwrap().method == "GET");
    }
}
