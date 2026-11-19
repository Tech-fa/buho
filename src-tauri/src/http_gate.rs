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

pub fn browser_calls_api(url: &Url, api_origin: Option<&str>, page: Option<&str>) -> bool {
    if looks_like_api(url) {
        return true;
    }
    let Some(api_origin) = api_origin else {
        return false;
    };
    let Ok(api) = Url::parse(api_origin) else {
        return false;
    };
    if origin_of(url) != origin_of(&api) {
        return false;
    }
    match page.and_then(|value| Url::parse(value).ok()) {
        Some(page) if origin_of(&page) == origin_of(&api) => false,
        _ => true,
    }
}

fn looks_like_api(url: &Url) -> bool {
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    let path = url.path().to_ascii_lowercase();
    host.starts_with("api.")
        || host.contains("-api.")
        || host.starts_with("api-")
        || path == "/api"
        || path.starts_with("/api/")
        || path.contains("/api/")
        || path.starts_with("/graphql")
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

/// A write can change what the open page is showing.
pub fn writes_data(method: &str) -> bool {
    matches!(method, "POST" | "PUT" | "PATCH" | "DELETE")
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
    let parsed = resolve_curl(command)?;
    request(&parsed, origins)
}

const RETRY_WITH_REQUEST: &str = "http.curl sends one HTTP request and does not run a shell. Retry this call with http.request, and put the JSON in body. Do not tell the user this call failed.";

/// A curl command, or a shell script wrapped around one HTTP request.
pub fn resolve_curl(command: &str) -> Result<HttpRequest, String> {
    if let Ok(req) = parse_curl(command) {
        if is_http_url(&req.url) && !command_needs_salvage(command) {
            return Ok(req);
        }
    }
    salvage_request(command)
}

fn is_http_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn command_needs_salvage(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    !lower.trim_start().starts_with("curl")
        || command.contains("<<")
        || lower.contains("python")
        || command.contains("@-")
        || command.contains("@/")
}

fn salvage_request(command: &str) -> Result<HttpRequest, String> {
    let url = first_http_url(command).ok_or_else(|| RETRY_WITH_REQUEST.to_string())?;
    let headers = collect_headers(command);
    let body = heredoc_json(command).or_else(|| inline_data(command));
    if body.as_deref().is_some_and(|value| value.starts_with('@')) {
        return Err(RETRY_WITH_REQUEST.into());
    }
    let method = explicit_method(command)
        .unwrap_or_else(|| {
            if body.is_some() {
                "POST".into()
            } else {
                "GET".into()
            }
        })
        .to_uppercase();
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

fn first_http_url(command: &str) -> Option<String> {
    request_call_url(command).or_else(|| url_outside_heredoc(command))
}

fn request_call_url(command: &str) -> Option<String> {
    let lower = command.to_ascii_lowercase();
    for marker in ["request(", "urlopen("] {
        let mut from = 0;
        while let Some(index) = lower[from..].find(marker) {
            let abs = from + index + marker.len();
            let after = command[abs..].trim_start();
            if let Some((value, _)) = read_arg(after) {
                if is_http_url(&value) {
                    return Some(value);
                }
            }
            from = abs;
        }
    }
    None
}

fn url_outside_heredoc(command: &str) -> Option<String> {
    let body = heredoc_span(command);
    let lower = command.to_ascii_lowercase();
    let mut from = 0;
    while from < command.len() {
        let start = ["https://", "http://"]
            .into_iter()
            .filter_map(|prefix| lower[from..].find(prefix).map(|index| from + index))
            .min();
        let Some(start) = start else {
            break;
        };
        let url = slice_url(&command[start..]);
        let inside = body.is_some_and(|(begin, finish)| start >= begin && start < finish);
        if !inside && is_http_url(&url) {
            return Some(url);
        }
        from = start + "http".len();
    }
    None
}

fn slice_url(rest: &str) -> String {
    let end = rest
        .find(|ch: char| {
            ch.is_whitespace() || matches!(ch, '\'' | '"' | '\\' | ')' | ',' | '<' | '>' | '`')
        })
        .unwrap_or(rest.len());
    rest[..end].trim_end_matches(['.', ';']).to_string()
}

fn heredoc_span(command: &str) -> Option<(usize, usize)> {
    let marker = command.find("<<")?;
    let after_at = marker + 2;
    let after = command[after_at..].trim_start();
    let trimmed = after_at + (command[after_at..].len() - after.len());
    let (delim, rest_at) = if let Some(rest) = after.strip_prefix('\'') {
        let end = rest.find('\'')?;
        (rest[..end].to_string(), trimmed + 1 + end + 1)
    } else if let Some(rest) = after.strip_prefix('"') {
        let end = rest.find('"')?;
        (rest[..end].to_string(), trimmed + 1 + end + 1)
    } else {
        let end = after.find(char::is_whitespace).unwrap_or(after.len());
        if end == 0 {
            return None;
        }
        (after[..end].to_string(), trimmed + end)
    };
    if delim.is_empty() {
        return None;
    }
    let rest = &command[rest_at..];
    let body_at = rest_at + (rest.len() - rest.trim_start_matches(['\r', '\n']).len());
    let body = &command[body_at..];
    let end = body
        .find(&format!("\n{delim}"))
        .or_else(|| body.find(&format!("\r\n{delim}")))?;
    Some((body_at, body_at + end))
}

fn explicit_method(command: &str) -> Option<String> {
    if let Some(value) = flag_value(command, "-X").or_else(|| flag_value(command, "--request")) {
        if !value.is_empty() {
            return Some(value);
        }
    }
    let mut search_from = 0;
    while let Some(index) = command[search_from..].to_ascii_lowercase().find("method") {
        let abs = search_from + index;
        let before_ok = abs == 0 || !command.as_bytes()[abs - 1].is_ascii_alphanumeric();
        let after = command[abs + "method".len()..].trim_start();
        if before_ok && let Some(after) = after.strip_prefix('=') {
            let after = after.trim_start();
            if let Some((value, _)) = read_arg(after) {
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }
        search_from = abs + "method".len();
    }
    None
}

fn collect_headers(command: &str) -> Vec<(String, String)> {
    let mut headers = flag_headers(command);
    for (name, value) in dict_headers(command) {
        if headers
            .iter()
            .any(|(existing, _)| existing.eq_ignore_ascii_case(&name))
        {
            continue;
        }
        headers.push((name, value));
    }
    headers
}

fn flag_headers(command: &str) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    let mut search_from = 0;
    while search_from < command.len() {
        let rest = &command[search_from..];
        let Some(index) = rest.find("-H").or_else(|| rest.find("--header")) else {
            break;
        };
        let abs = search_from + index;
        let flag = if rest[index..].starts_with("--header") {
            "--header"
        } else {
            "-H"
        };
        let before_ok = abs == 0 || command.as_bytes()[abs - 1].is_ascii_whitespace();
        let after = &command[abs + flag.len()..];
        if before_ok && (after.starts_with(char::is_whitespace) || after.starts_with(['\'', '"'])) {
            let after = after.trim_start();
            if let Some((raw, consumed)) = read_arg(after) {
                if let Some((name, value)) = split_header(&raw) {
                    headers.push((name, value));
                }
                search_from = command.len() - after.len() + consumed;
                continue;
            }
        }
        search_from = abs + flag.len();
    }
    headers
}

fn dict_headers(command: &str) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    let mut search_from = 0;
    let lower = command.to_ascii_lowercase();
    while let Some(index) = lower[search_from..].find("headers") {
        let abs = search_from + index;
        let after = command[abs + "headers".len()..].trim_start();
        search_from = abs + "headers".len();
        let Some(after) = after.strip_prefix('=') else {
            continue;
        };
        let after = after.trim_start();
        let Some(body) = after.strip_prefix('{') else {
            continue;
        };
        let Some(end) = close_delim(body, '{', '}') else {
            continue;
        };
        let pairs = quoted_pieces(&body[..end]);
        let mut index = 0;
        while index + 1 < pairs.len() {
            let name = pairs[index].trim().to_string();
            let value = pairs[index + 1].trim().to_string();
            if let Some((name, value)) = split_header(&format!("{name}: {value}")) {
                headers.push((name, value));
            }
            index += 2;
        }
    }
    headers
}

fn split_header(raw: &str) -> Option<(String, String)> {
    let (name, value) = raw.split_once(':')?;
    let name = name.trim();
    let value = value.trim();
    if name.is_empty()
        || value.is_empty()
        || name.contains(['\r', '\n', ':'])
        || value.contains(['\r', '\n'])
    {
        return None;
    }
    Some((name.to_string(), value.to_string()))
}

fn heredoc_json(command: &str) -> Option<String> {
    let marker = command.find("<<")?;
    let after = command[marker + 2..].trim_start();
    let (delim, rest) = if let Some(rest) = after.strip_prefix('\'') {
        let end = rest.find('\'')?;
        (rest[..end].to_string(), &rest[end + 1..])
    } else if let Some(rest) = after.strip_prefix('"') {
        let end = rest.find('"')?;
        (rest[..end].to_string(), &rest[end + 1..])
    } else {
        let end = after.find(char::is_whitespace).unwrap_or(after.len());
        if end == 0 {
            return None;
        }
        (after[..end].to_string(), &after[end..])
    };
    if delim.is_empty() || delim.contains('\n') {
        return None;
    }
    let rest = rest.trim_start_matches(['\r', '\n']);
    let end = rest
        .find(&format!("\n{delim}"))
        .or_else(|| rest.find(&format!("\r\n{delim}")))?;
    let body = rest[..end].trim();
    if body.starts_with('{') || body.starts_with('[') {
        Some(body.to_string())
    } else {
        None
    }
}

fn inline_data(command: &str) -> Option<String> {
    for flag in ["--data-binary", "--data-raw", "--data", "-d"] {
        if let Some(value) = flag_value(command, flag) {
            if !value.starts_with('@') && !value.is_empty() {
                return Some(value);
            }
        }
    }
    largest_json(command)
}

fn largest_json(command: &str) -> Option<String> {
    let mut best: Option<String> = None;
    for (index, ch) in command.char_indices() {
        if ch != '{' && ch != '[' {
            continue;
        }
        let close = if ch == '{' { '}' } else { ']' };
        let rest = &command[index + ch.len_utf8()..];
        let Some(end) = close_delim(rest, ch, close) else {
            continue;
        };
        let slice = &command[index..index + ch.len_utf8() + end + close.len_utf8()];
        if serde_json::from_str::<Value>(slice).is_err() {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|current| slice.len() > current.len())
        {
            best = Some(slice.to_string());
        }
    }
    best
}

fn flag_value(command: &str, flag: &str) -> Option<String> {
    let mut search_from = 0;
    while search_from < command.len() {
        let Some(index) = command[search_from..].find(flag) else {
            return None;
        };
        let abs = search_from + index;
        let before_ok = abs == 0 || command.as_bytes()[abs - 1].is_ascii_whitespace();
        let after = &command[abs + flag.len()..];
        let long = flag.starts_with("--");
        let after_ok = if long {
            after.starts_with(char::is_whitespace) || after.starts_with(['\'', '"', '='])
        } else {
            after.starts_with(char::is_whitespace) || after.starts_with(['\'', '"'])
        };
        if before_ok && after_ok {
            let after = after.trim_start().trim_start_matches('=');
            if let Some((value, _)) = read_arg(after) {
                return Some(value);
            }
        }
        search_from = abs + flag.len();
    }
    None
}

fn read_arg(input: &str) -> Option<(String, usize)> {
    let mut chars = input.char_indices();
    let (_, first) = chars.next()?;
    if first == '\'' || first == '"' {
        let mut current = String::new();
        while let Some((index, ch)) = chars.next() {
            if ch == '\\' {
                if let Some((_, next)) = chars.next() {
                    current.push(next);
                }
                continue;
            }
            if ch == first {
                return Some((current, index + ch.len_utf8()));
            }
            current.push(ch);
        }
        return None;
    }
    let end = input.find(char::is_whitespace).unwrap_or(input.len());
    if end == 0 {
        return None;
    }
    Some((input[..end].to_string(), end))
}

fn quoted_pieces(input: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\'' && ch != '"' {
            continue;
        }
        let mut current = String::new();
        while let Some(next) = chars.next() {
            if next == '\\' {
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                }
                continue;
            }
            if next == ch {
                break;
            }
            current.push(next);
        }
        pieces.push(current);
    }
    pieces
}

fn close_delim(input: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 1;
    let mut chars = input.char_indices();
    let mut quote: Option<char> = None;
    while let Some((index, ch)) = chars.next() {
        if let Some(active) = quote {
            if ch == '\\' {
                chars.next();
                continue;
            }
            if ch == active {
                quote = None;
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
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
    fn the_browser_cannot_open_an_api() {
        let api = Url::parse("https://api.example.com/items").unwrap();
        let nested = Url::parse("https://app.example.com/api/items").unwrap();
        let fourss = Url::parse("https://4ss-app-api.example.com/users").unwrap();
        let page = Url::parse("https://app.example.com/users").unwrap();
        let local_api = Url::parse("http://203.0.113.10:3012/users").unwrap();
        assert!(browser_calls_api(&api, None, None));
        assert!(browser_calls_api(
            &nested,
            None,
            Some("https://app.example.com")
        ));
        assert!(browser_calls_api(&fourss, None, None));
        assert!(!browser_calls_api(
            &page,
            None,
            Some("https://app.example.com")
        ));
        assert!(browser_calls_api(
            &local_api,
            Some("http://203.0.113.10:3012"),
            Some("https://app.example.com")
        ));
        assert!(!browser_calls_api(
            &page,
            Some("https://app.example.com"),
            Some("https://app.example.com/home")
        ));
    }

    #[test]
    fn only_writes_reload_a_page() {
        assert!(writes_data("POST"));
        assert!(writes_data("PUT"));
        assert!(writes_data("PATCH"));
        assert!(writes_data("DELETE"));
        assert!(!writes_data("GET"));
        assert!(!writes_data("HEAD"));
        assert!(!writes_data("OPTIONS"));
    }

    #[test]
    fn a_shell_script_around_one_request_is_sent() {
        let command = r#"cat > /tmp/prop.json <<'JSON'
{"inputs":[],"steps":[{"id":"create-app","title":"Open","link":{"url":"https://api.slack.com/apps"}}]}
JSON
python3 -c "
r=urllib.request.Request('http://localhost:3004/integrations/abc/proposal',data=body.encode(),method='POST',headers={'Authorization':'Bearer secret','Content-Type':'application/json'})
""#;
        let req = resolve_curl(command).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.url, "http://localhost:3004/integrations/abc/proposal");
        assert_eq!(
            req.body.as_deref(),
            Some(
                r#"{"inputs":[],"steps":[{"id":"create-app","title":"Open","link":{"url":"https://api.slack.com/apps"}}]}"#
            )
        );
        assert!(
            req.headers
                .iter()
                .any(|(name, value)| { name == "Authorization" && value == "Bearer secret" })
        );
        assert!(
            req.headers
                .iter()
                .any(|(name, value)| { name == "Content-Type" && value == "application/json" })
        );
    }

    #[test]
    fn a_curl_heredoc_uses_the_document_as_the_body() {
        let command = r#"curl -s -X POST "http://localhost:3004/integrations/abc/proposal" -H "Authorization: Bearer secret" -H "Content-Type: application/json" -d @- <<'JSON'
{"ok":true}
JSON"#;
        let req = resolve_curl(command).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.url, "http://localhost:3004/integrations/abc/proposal");
        assert_eq!(req.body.as_deref(), Some(r#"{"ok":true}"#));
        assert!(
            req.headers
                .iter()
                .any(|(name, value)| name == "Authorization" && value == "Bearer secret")
        );
    }

    #[test]
    fn a_plain_curl_is_unchanged_and_a_script_without_a_url_asks_for_a_retry() {
        let plain = resolve_curl("curl -X GET https://api.example.com/items").unwrap();
        assert_eq!(plain.method, "GET");
        assert_eq!(plain.url, "https://api.example.com/items");
        let err = resolve_curl("python3 -c \"print('hello')\"").unwrap_err();
        assert!(err.contains("http.request"));
        assert!(err.contains("Do not tell the user"));
    }

    #[test]
    fn rejects_header_injection_and_unknown_methods() {
        assert!(parse_curl("curl https://example.com -H 'X-A: one\r\nX-B: two'").is_err());
        assert!(parse_curl("curl https://example.com -X TRACE").is_err());
        assert!(parse_curl("curl 'https://example.com'").unwrap().method == "GET");
    }
}
