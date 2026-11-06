use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

#[derive(Debug, Default, Clone)]
pub struct Usage {
    pub requests: u64,
    pub prompt_tokens: u64,
    pub cached_tokens: u64,
    pub completion_tokens: u64,
    /// `None` unless the provider reports cost (OpenRouter does).
    pub cost: Option<f64>,
}

impl Usage {
    fn add(&mut self, usage: &Value) {
        self.requests += 1;
        let n = |v: &Value| v.as_u64().unwrap_or(0);
        self.prompt_tokens += n(&usage["prompt_tokens"]);
        self.cached_tokens += n(&usage["prompt_tokens_details"]["cached_tokens"]);
        self.completion_tokens += n(&usage["completion_tokens"]);
        if let Some(cost) = usage["cost"].as_f64() {
            *self.cost.get_or_insert(0.0) += cost;
        }
    }
}

pub struct ChatRequest<'a> {
    pub messages: &'a [Value],
    /// Tool definitions in OpenAI format, or `None` for no tools.
    pub tools: Option<Value>,
    /// Whether the model may call tools this turn (`tool_choice` auto vs none).
    pub tools_allowed: bool,
    /// Name and JSON schema the final answer must match.
    pub schema: Option<(&'a str, Value)>,
}

pub struct ChatClient {
    http: reqwest::blocking::Client,
    base_url: String,
    api_key: String,
    model: String,
    effort: Option<String>,
    usage: Usage,
}

impl ChatClient {
    pub fn new(
        base_url: String,
        api_key: String,
        model: String,
        effort: Option<String>,
    ) -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()?;
        Ok(Self {
            http,
            base_url,
            api_key,
            model,
            effort,
            usage: Usage::default(),
        })
    }

    /// Totals across every request made by this client.
    pub fn usage(&self) -> &Usage {
        &self.usage
    }

    /// Sends one chat completion and returns the assistant message.
    pub fn chat(&mut self, req: ChatRequest) -> Result<Value> {
        let mut body = json!({
            "model": self.model,
            "max_tokens": 16000,
            "messages": req.messages,
        });
        if let Some(tools) = req.tools {
            body["tools"] = tools;
            body["tool_choice"] = json!(if req.tools_allowed { "auto" } else { "none" });
        }
        if let Some((name, schema)) = req.schema {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": { "name": name, "strict": true, "schema": schema },
            });
        }
        if let Some(effort) = &self.effort {
            // OpenRouter-specific
            body["reasoning"] = json!({ "effort": effort });
        }

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .header("X-Title", "sidekick")
            .json(&body)
            .send()
            .with_context(|| format!("request to {url} failed"))?;

        let status = resp.status();
        let mut value: Value = resp.json().context("API returned non-JSON")?;
        if !status.is_success() || value.get("error").is_some() {
            let msg = value["error"]["message"]
                .as_str()
                .unwrap_or("unknown error");
            bail!("API error ({status}): {msg}");
        }

        self.usage.add(&value["usage"]);

        let choice = &mut value["choices"][0];
        match choice["finish_reason"].as_str() {
            Some("length") => bail!("response was cut off (max_tokens)"),
            Some("content_filter") => bail!("the model declined the request"),
            _ => {}
        }
        Ok(choice["message"].take())
    }
}
