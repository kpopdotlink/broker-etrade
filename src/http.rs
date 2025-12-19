//! HTTP host function wrapper for WASM plugins

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashMap;
use std::slice;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u32,
}

fn default_timeout() -> u32 {
    30000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
    pub error: Option<String>,
}

impl HttpResponse {
    pub fn is_success(&self) -> bool {
        self.error.is_none() && (200..300).contains(&self.status)
    }

    pub fn json<T: DeserializeOwned>(&self) -> Result<T, String> {
        serde_json::from_str(&self.body).map_err(|e| format!("JSON parse error: {}", e))
    }
}

extern "C" {
    fn http_request(req_ptr: i32, req_len: i32) -> u64;
}

pub fn execute(request: HttpRequest) -> HttpResponse {
    let req_bytes = serde_json::to_vec(&request).expect("Failed to serialize request");
    let req_ptr = req_bytes.as_ptr() as i32;
    let req_len = req_bytes.len() as i32;

    let result_packed = unsafe { http_request(req_ptr, req_len) };

    let res_ptr = (result_packed >> 32) as i32;
    let res_len = (result_packed & 0xFFFFFFFF) as i32;

    if res_ptr == 0 || res_len == 0 {
        return HttpResponse {
            status: 0,
            headers: HashMap::new(),
            body: String::new(),
            error: Some("Host function returned null response".to_string()),
        };
    }

    let res_slice = unsafe { slice::from_raw_parts(res_ptr as *const u8, res_len as usize) };

    serde_json::from_slice(res_slice).unwrap_or_else(|e| HttpResponse {
        status: 0,
        headers: HashMap::new(),
        body: String::new(),
        error: Some(format!("Failed to parse response: {}", e)),
    })
}

pub struct HttpClient {
    base_url: String,
    default_headers: HashMap<String, String>,
    timeout_ms: u32,
}

impl HttpClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            default_headers: HashMap::new(),
            timeout_ms: 30000,
        }
    }

    pub fn header(mut self, key: &str, value: &str) -> Self {
        self.default_headers.insert(key.to_string(), value.to_string());
        self
    }

    pub fn set_header(&mut self, key: &str, value: &str) {
        self.default_headers.insert(key.to_string(), value.to_string());
    }

    fn build_url(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else {
            format!("{}{}", self.base_url, path)
        }
    }

    fn merge_headers(&self, extra: Option<HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = self.default_headers.clone();
        if let Some(extra) = extra {
            headers.extend(extra);
        }
        headers
    }

    pub fn get(&self, path: &str, headers: Option<HashMap<String, String>>) -> HttpResponse {
        execute(HttpRequest {
            method: HttpMethod::Get,
            url: self.build_url(path),
            headers: self.merge_headers(headers),
            body: None,
            timeout_ms: self.timeout_ms,
        })
    }

    pub fn post(
        &self,
        path: &str,
        body: Option<String>,
        headers: Option<HashMap<String, String>>,
    ) -> HttpResponse {
        execute(HttpRequest {
            method: HttpMethod::Post,
            url: self.build_url(path),
            headers: self.merge_headers(headers),
            body,
            timeout_ms: self.timeout_ms,
        })
    }

    pub fn post_json<T: Serialize>(
        &self,
        path: &str,
        body: &T,
        headers: Option<HashMap<String, String>>,
    ) -> HttpResponse {
        let body_str = serde_json::to_string(body).expect("Failed to serialize body");
        let mut h = self.merge_headers(headers);
        h.entry("Content-Type".to_string())
            .or_insert_with(|| "application/json".to_string());
        execute(HttpRequest {
            method: HttpMethod::Post,
            url: self.build_url(path),
            headers: h,
            body: Some(body_str),
            timeout_ms: self.timeout_ms,
        })
    }
}
