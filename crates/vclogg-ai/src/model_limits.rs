//! Optional model metadata discovery. Missing metadata never invents a limit.
use crate::{Protocol, ProviderConfig};
use anyhow::{Context, Result};
use serde_json::Value;
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelLimits {
    pub context_window_tokens: Option<u32>,
    pub max_output_tokens: Option<u32>,
}

fn positive(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .filter(|number| *number > 0)
        .and_then(|number| u32::try_from(number).ok())
}

pub fn parse_model_limits(protocol: Protocol, value: &Value) -> ModelLimits {
    let context_window_tokens = match protocol {
        Protocol::Anthropic => positive(&value["max_input_tokens"]),
        Protocol::OpenAi => positive(&value["context_length"])
            .or_else(|| positive(&value["context_window"]))
            .or_else(|| positive(&value["max_model_len"])),
    };
    let max_output_tokens = match protocol {
        Protocol::Anthropic => positive(&value["max_tokens"]),
        Protocol::OpenAi => positive(&value["max_output_tokens"]),
    };
    ModelLimits {
        context_window_tokens,
        max_output_tokens,
    }
}

pub async fn discover_model_limits(config: &ProviderConfig) -> Result<ModelLimits> {
    let mut url = url::Url::parse(&config.base_url).context("Invalid API base URL")?;
    config.endpoint()?;
    if config.protocol == Protocol::OpenAi && url.host_str() == Some("api.openai.com") {
        return Ok(ModelLimits::default());
    }
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("Invalid API base URL"))?
        .pop_if_empty()
        .push("models")
        .push(&config.model);
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut request = client.get(url);
    match config.protocol {
        Protocol::Anthropic => {
            let mut key = reqwest::header::HeaderValue::from_str(&config.api_key)
                .context("Invalid API key")?;
            key.set_sensitive(true);
            request = request
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01");
        }
        Protocol::OpenAi => {
            if !config.api_key.is_empty() {
                request = request.bearer_auth(&config.api_key);
            }
        }
    }
    let response = request.send().await.context("Model metadata unavailable")?;
    if !response.status().is_success() {
        anyhow::bail!(
            "Model metadata unavailable (HTTP {})",
            response.status().as_u16()
        );
    }
    let bytes = response.bytes().await?;
    if bytes.len() > 64 * 1024 {
        anyhow::bail!("Model metadata response is too large");
    }
    let body: Value = serde_json::from_slice(&bytes).context("Invalid model metadata")?;
    Ok(parse_model_limits(config.protocol, &body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn reads_only_advertised_limits() {
        assert_eq!(
            parse_model_limits(
                Protocol::Anthropic,
                &json!({"max_input_tokens":200000,"max_tokens":64000})
            ),
            ModelLimits {
                context_window_tokens: Some(200000),
                max_output_tokens: Some(64000)
            }
        );
        assert_eq!(
            parse_model_limits(Protocol::OpenAi, &json!({"id":"any"})),
            ModelLimits::default()
        );
        assert_eq!(
            parse_model_limits(
                Protocol::OpenAi,
                &json!({"context_length":131072,"max_output_tokens":8192})
            )
            .context_window_tokens,
            Some(131072)
        );
    }
}
