//! API error detection utilities for various AI providers.
//!
//! This module provides centralized error detection for:
//! - Anthropic/Claude
//! - OpenAI/Codex
//! - Alibaba Cloud/QWen
//! - Azure OpenAI
//! - Google AI/Gemini
//! - DeepSeek
//!
//! The detection logic matches error patterns in API responses and logs
//! to identify quota exhaustion, rate limiting, server overload, and auth failures.

use super::NormalizedEntryError;

/// Detected API error with categorization
#[derive(Debug, Clone, PartialEq)]
pub struct DetectedApiError {
    pub error_type: NormalizedEntryError,
    pub message: String,
}

/// Detect API errors from message content.
///
/// Returns a categorized error if a known error pattern is detected,
/// or None if the content doesn't match any known error patterns.
pub fn detect_api_error(content: &str) -> Option<DetectedApiError> {
    let lowered = content.to_lowercase();

    // === Anthropic/Claude specific errors ===
    if lowered.contains("anthropic") || lowered.contains("claude") {
        if lowered.contains("credit balance") || lowered.contains("credit exhausted") {
            return Some(DetectedApiError {
                error_type: NormalizedEntryError::QuotaExceeded {
                    provider: Some("Anthropic".to_string()),
                },
                message: "Claude credit balance exhausted".to_string(),
            });
        }
        if lowered.contains("rate limit") || lowered.contains("rate_limit") {
            return Some(DetectedApiError {
                error_type: NormalizedEntryError::RateLimitExceeded {
                    provider: Some("Anthropic".to_string()),
                },
                message: "Claude API rate limit exceeded".to_string(),
            });
        }
        if lowered.contains("overloaded") {
            return Some(DetectedApiError {
                error_type: NormalizedEntryError::ServerOverloaded {
                    provider: Some("Anthropic".to_string()),
                },
                message: "Claude API is overloaded".to_string(),
            });
        }
    }

    // === OpenAI/Codex specific errors ===
    if lowered.contains("openai")
        || lowered.contains("codex")
        || lowered.contains("gpt-4")
        || lowered.contains("gpt-3")
        || lowered.contains("o1-")
        || lowered.contains("o3-")
    {
        if lowered.contains("billing hard limit")
            || lowered.contains("exceeded your current quota")
            || lowered.contains("insufficient_quota")
        {
            return Some(DetectedApiError {
                error_type: NormalizedEntryError::QuotaExceeded {
                    provider: Some("OpenAI".to_string()),
                },
                message: "OpenAI quota exceeded".to_string(),
            });
        }
        if lowered.contains("rate limit") || lowered.contains("rate_limit_exceeded") {
            return Some(DetectedApiError {
                error_type: NormalizedEntryError::RateLimitExceeded {
                    provider: Some("OpenAI".to_string()),
                },
                message: "OpenAI API rate limit exceeded".to_string(),
            });
        }
        if lowered.contains("context_length_exceeded") || lowered.contains("maximum context length")
        {
            return Some(DetectedApiError {
                error_type: NormalizedEntryError::ContextLimitExceeded {
                    provider: Some("OpenAI".to_string()),
                },
                message: "OpenAI context length exceeded".to_string(),
            });
        }
        if lowered.contains("invalid_api_key") || lowered.contains("incorrect api key") {
            return Some(DetectedApiError {
                error_type: NormalizedEntryError::AuthenticationFailed {
                    provider: Some("OpenAI".to_string()),
                },
                message: "OpenAI API key invalid".to_string(),
            });
        }
    }

    // === Alibaba Cloud / QWen Coder specific errors ===
    if lowered.contains("qwen")
        || lowered.contains("tongyi")
        || lowered.contains("dashscope")
        || lowered.contains("aliyun")
        || lowered.contains("alibaba")
    {
        if lowered.contains("quota")
            || lowered.contains("余额不足")
            || lowered.contains("账户余额")
            || lowered.contains("免费额度")
        {
            return Some(DetectedApiError {
                error_type: NormalizedEntryError::QuotaExceeded {
                    provider: Some("Alibaba".to_string()),
                },
                message: "QWen API 额度已用尽".to_string(),
            });
        }
        if lowered.contains("rate limit")
            || lowered.contains("限流")
            || lowered.contains("请求过于频繁")
            || lowered.contains("qps")
        {
            return Some(DetectedApiError {
                error_type: NormalizedEntryError::RateLimitExceeded {
                    provider: Some("Alibaba".to_string()),
                },
                message: "QWen API 请求频率超限".to_string(),
            });
        }
        if lowered.contains("accessdenied") || lowered.contains("invalidaccesskey") {
            return Some(DetectedApiError {
                error_type: NormalizedEntryError::AuthenticationFailed {
                    provider: Some("Alibaba".to_string()),
                },
                message: "QWen API 密钥无效".to_string(),
            });
        }
    }

    // === Azure OpenAI specific errors ===
    if lowered.contains("azure") && lowered.contains("openai") {
        if lowered.contains("quota") || lowered.contains("tokens per minute") {
            return Some(DetectedApiError {
                error_type: NormalizedEntryError::QuotaExceeded {
                    provider: Some("Azure".to_string()),
                },
                message: "Azure OpenAI quota exceeded".to_string(),
            });
        }
        if lowered.contains("rate limit") || lowered.contains("429") {
            return Some(DetectedApiError {
                error_type: NormalizedEntryError::RateLimitExceeded {
                    provider: Some("Azure".to_string()),
                },
                message: "Azure OpenAI rate limit exceeded".to_string(),
            });
        }
    }

    // === Google AI / Gemini specific errors ===
    if lowered.contains("google")
        || lowered.contains("gemini")
        || lowered.contains("palm")
        || lowered.contains("vertex")
    {
        if lowered.contains("quota") || lowered.contains("resource_exhausted") {
            return Some(DetectedApiError {
                error_type: NormalizedEntryError::QuotaExceeded {
                    provider: Some("Google".to_string()),
                },
                message: "Google AI quota exceeded".to_string(),
            });
        }
        if lowered.contains("rate limit") || lowered.contains("429") {
            return Some(DetectedApiError {
                error_type: NormalizedEntryError::RateLimitExceeded {
                    provider: Some("Google".to_string()),
                },
                message: "Google AI rate limit exceeded".to_string(),
            });
        }
    }

    // === DeepSeek specific errors ===
    if lowered.contains("deepseek") {
        if lowered.contains("quota") || lowered.contains("balance") {
            return Some(DetectedApiError {
                error_type: NormalizedEntryError::QuotaExceeded {
                    provider: Some("DeepSeek".to_string()),
                },
                message: "DeepSeek quota or credit limit reached".to_string(),
            });
        }
        if lowered.contains("rate limit") || lowered.contains("429") {
            return Some(DetectedApiError {
                error_type: NormalizedEntryError::RateLimitExceeded {
                    provider: Some("DeepSeek".to_string()),
                },
                message: "DeepSeek API rate limit exceeded".to_string(),
            });
        }
    }

    // === Generic quota/credit exhaustion (fallback) ===
    if lowered.contains("quota exceeded")
        || lowered.contains("quota_exceeded")
        || lowered.contains("credit balance")
        || lowered.contains("credit exhausted")
        || lowered.contains("insufficient credit")
        || lowered.contains("insufficient_quota")
        || (lowered.contains("billing") && lowered.contains("limit"))
        || lowered.contains("余额不足")
        || (lowered.contains("额度") && (lowered.contains("用尽") || lowered.contains("不足")))
    {
        return Some(DetectedApiError {
            error_type: NormalizedEntryError::QuotaExceeded { provider: None },
            message: "API quota or credit limit reached".to_string(),
        });
    }

    // === Generic rate limiting (fallback) ===
    if lowered.contains("rate limit")
        || lowered.contains("rate_limit")
        || lowered.contains("too many requests")
        || lowered.contains("请求过于频繁")
        || lowered.contains("限流")
    {
        return Some(DetectedApiError {
            error_type: NormalizedEntryError::RateLimitExceeded { provider: None },
            message: "API rate limit exceeded".to_string(),
        });
    }

    // === Generic server overload (fallback) ===
    if lowered.contains("overloaded")
        || lowered.contains("server is busy")
        || lowered.contains("503")
        || lowered.contains("service unavailable")
        || lowered.contains("服务繁忙")
        || lowered.contains("系统繁忙")
    {
        return Some(DetectedApiError {
            error_type: NormalizedEntryError::ServerOverloaded { provider: None },
            message: "API server is overloaded".to_string(),
        });
    }

    // === Generic authentication errors (fallback) ===
    if lowered.contains("invalid api key")
        || lowered.contains("invalid_api_key")
        || lowered.contains("authentication failed")
        || lowered.contains("unauthorized")
        || lowered.contains("401")
        || lowered.contains("密钥无效")
        || lowered.contains("认证失败")
    {
        return Some(DetectedApiError {
            error_type: NormalizedEntryError::AuthenticationFailed { provider: None },
            message: "API authentication failed".to_string(),
        });
    }

    // === Context/token limit errors (fallback) ===
    if lowered.contains("context length")
        || lowered.contains("context_length")
        || lowered.contains("token limit")
        || lowered.contains("maximum tokens")
        || lowered.contains("上下文长度")
        || lowered.contains("超出最大")
    {
        return Some(DetectedApiError {
            error_type: NormalizedEntryError::ContextLimitExceeded { provider: None },
            message: "Context or token limit exceeded".to_string(),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_claude_quota() {
        let msg = "Error: Anthropic API returned: credit balance exhausted";
        let result = detect_api_error(msg);
        assert!(result.is_some());
        let err = result.unwrap();
        assert!(matches!(
            err.error_type,
            NormalizedEntryError::QuotaExceeded { provider: Some(p) } if p == "Anthropic"
        ));
    }

    #[test]
    fn test_detect_openai_rate_limit() {
        let msg = "OpenAI API error: rate limit exceeded";
        let result = detect_api_error(msg);
        assert!(result.is_some());
        let err = result.unwrap();
        assert!(matches!(
            err.error_type,
            NormalizedEntryError::RateLimitExceeded { provider: Some(p) } if p == "OpenAI"
        ));
    }

    #[test]
    fn test_detect_qwen_quota_chinese() {
        let msg = "DashScope 返回错误: 账户余额不足";
        let result = detect_api_error(msg);
        assert!(result.is_some());
        let err = result.unwrap();
        assert!(matches!(
            err.error_type,
            NormalizedEntryError::QuotaExceeded { provider: Some(p) } if p == "Alibaba"
        ));
    }

    #[test]
    fn test_detect_generic_overload() {
        let msg = "Error: The server is overloaded, please try again later";
        let result = detect_api_error(msg);
        assert!(result.is_some());
        let err = result.unwrap();
        assert!(matches!(
            err.error_type,
            NormalizedEntryError::ServerOverloaded { .. }
        ));
    }

    #[test]
    fn test_no_error_detected() {
        let msg = "Hello, how can I help you today?";
        let result = detect_api_error(msg);
        assert!(result.is_none());
    }
}
