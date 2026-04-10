use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct OllamaTagsResponse {
    pub models: Vec<OllamaModel>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct OllamaModel {
    pub name: String,
    pub size: u64,
    #[serde(default)]
    pub parameter_size: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct OllamaShowResponse {
    pub modelfile: Option<String>,
    #[serde(default)]
    pub parameters: Option<String>,
    #[serde(default)]
    pub details: Option<OllamaModelDetails>,
    #[serde(default)]
    pub model_info: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct OllamaModelDetails {
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub parameter_size: Option<String>,
    #[serde(default)]
    pub quantization_level: Option<String>,
}

pub struct OllamaClient {
    base_url: String,
}

/// Parse parameter size strings like "8B", "7.2B", "70B", "3.8B" into billions.
pub fn parse_parameter_size(s: &str) -> Option<f64> {
    let s = s.trim().to_uppercase();
    if let Some(num_str) = s.strip_suffix('B') {
        num_str.trim().parse::<f64>().ok()
    } else {
        s.parse::<f64>().ok()
    }
}

impl OllamaClient {
    pub fn new(base_url: Option<&str>) -> Self {
        Self {
            base_url: base_url
                .unwrap_or("http://localhost:11434")
                .trim_end_matches('/')
                .to_string(),
        }
    }

    pub fn ping(&self) -> bool {
        ureq::get(&self.base_url)
            .timeout(std::time::Duration::from_secs(5))
            .call()
            .is_ok()
    }

    pub fn list_models(&self) -> Result<Vec<OllamaModel>, String> {
        let url = format!("{}/api/tags", self.base_url);
        let response = ureq::get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .call()
            .map_err(|e| format!("Failed to connect to Ollama: {}", e))?;

        let tags: OllamaTagsResponse = response
            .into_json()
            .map_err(|e| format!("Failed to parse Ollama response: {}", e))?;

        Ok(tags.models)
    }

    /// Get detailed info about a model via POST /api/show.
    /// Works for both locally installed models and models in the Ollama registry.
    pub fn show_model(&self, name: &str) -> Result<OllamaShowResponse, String> {
        let url = format!("{}/api/show", self.base_url);
        let body = serde_json::json!({ "name": name });

        let response = ureq::post(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send_json(&body)
            .map_err(|e| format!("Failed to get model info from Ollama: {}", e))?;

        let show: OllamaShowResponse = response
            .into_json()
            .map_err(|e| format!("Failed to parse model info: {}", e))?;

        Ok(show)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tags_response() {
        let json = r#"{"models":[{"name":"llama3:8b","size":4661224676,"parameter_size":"8B"},{"name":"mistral:latest","size":4109854934}]}"#;
        let resp: OllamaTagsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.models.len(), 2);
        assert_eq!(resp.models[0].name, "llama3:8b");
        assert_eq!(resp.models[0].size, 4661224676);
        assert_eq!(resp.models[0].parameter_size, Some("8B".to_string()));
        assert_eq!(resp.models[1].name, "mistral:latest");
        assert_eq!(resp.models[1].parameter_size, None);
    }

    #[test]
    fn test_client_creation() {
        let client = OllamaClient::new(None);
        assert_eq!(client.base_url, "http://localhost:11434");

        let client = OllamaClient::new(Some("http://example.com:1234/"));
        assert_eq!(client.base_url, "http://example.com:1234");
    }

    #[test]
    fn test_parse_parameter_size() {
        assert_eq!(parse_parameter_size("8B"), Some(8.0));
        assert_eq!(parse_parameter_size("7.2B"), Some(7.2));
        assert_eq!(parse_parameter_size("70B"), Some(70.0));
        assert_eq!(parse_parameter_size("3.8b"), Some(3.8));
        assert_eq!(parse_parameter_size("  13B  "), Some(13.0));
        assert_eq!(parse_parameter_size("nope"), None);
    }

    #[test]
    fn test_parse_show_response() {
        let json = r#"{"modelfile":"...","details":{"format":"gguf","family":"llama","parameter_size":"8B","quantization_level":"Q4_K_M"}}"#;
        let resp: OllamaShowResponse = serde_json::from_str(json).unwrap();
        let details = resp.details.unwrap();
        assert_eq!(details.parameter_size, Some("8B".to_string()));
        assert_eq!(details.quantization_level, Some("Q4_K_M".to_string()));
        assert_eq!(details.family, Some("llama".to_string()));
    }

    #[test]
    fn test_parse_show_response_minimal() {
        let json = r#"{"modelfile":"..."}"#;
        let resp: OllamaShowResponse = serde_json::from_str(json).unwrap();
        assert!(resp.details.is_none());
    }
}
