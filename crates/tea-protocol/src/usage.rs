use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// Largest integer exactly representable by a JavaScript `Number`.
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_DECIMAL_DIGITS: usize = 36;
const MAX_DECIMAL_SCALE: usize = 18;

/// A token count safe to encode as a JSON number for JavaScript consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct TokenCount(u64);

impl TokenCount {
    /// Creates a JavaScript-safe token count.
    ///
    /// # Errors
    ///
    /// Returns [`UsageError::UnsafeInteger`] when `value` exceeds
    /// [`MAX_SAFE_INTEGER`].
    pub const fn new(value: u64) -> Result<Self, UsageError> {
        if value <= MAX_SAFE_INTEGER {
            Ok(Self(value))
        } else {
            Err(UsageError::UnsafeInteger)
        }
    }

    /// Returns the integer token count.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for TokenCount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u64::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Provider-neutral token usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_field_names)] // `_tokens` is stable domain and wire vocabulary.
pub struct Usage {
    input_tokens: TokenCount,
    output_tokens: TokenCount,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_read_tokens: Option<TokenCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_write_tokens: Option<TokenCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_tokens: Option<TokenCount>,
}

impl Usage {
    /// Creates usage with required input and output token counts.
    #[must_use]
    pub const fn new(input_tokens: TokenCount, output_tokens: TokenCount) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        }
    }

    /// Returns input tokens.
    #[must_use]
    pub const fn input_tokens(&self) -> TokenCount {
        self.input_tokens
    }

    /// Returns output tokens.
    #[must_use]
    pub const fn output_tokens(&self) -> TokenCount {
        self.output_tokens
    }

    /// Returns cache-read tokens when reported.
    #[must_use]
    pub const fn cache_read_tokens(&self) -> Option<TokenCount> {
        self.cache_read_tokens
    }

    /// Returns cache-write tokens when reported.
    #[must_use]
    pub const fn cache_write_tokens(&self) -> Option<TokenCount> {
        self.cache_write_tokens
    }

    /// Returns reasoning tokens when reported.
    #[must_use]
    pub const fn reasoning_tokens(&self) -> Option<TokenCount> {
        self.reasoning_tokens
    }

    /// Adds the cache-read token count.
    #[must_use]
    pub const fn with_cache_read(mut self, value: TokenCount) -> Self {
        self.cache_read_tokens = Some(value);
        self
    }

    /// Adds the cache-write token count.
    #[must_use]
    pub const fn with_cache_write(mut self, value: TokenCount) -> Self {
        self.cache_write_tokens = Some(value);
        self
    }

    /// Adds reasoning tokens, which must be a subset of output tokens.
    ///
    /// # Errors
    ///
    /// Returns [`UsageError::ReasoningExceedsOutput`] when the reasoning count
    /// exceeds the output count.
    pub fn with_reasoning(mut self, value: TokenCount) -> Result<Self, UsageError> {
        if value > self.output_tokens {
            return Err(UsageError::ReasoningExceedsOutput);
        }
        self.reasoning_tokens = Some(value);
        Ok(self)
    }

    /// Returns the total billable/context token count without double-counting reasoning.
    ///
    /// # Errors
    ///
    /// Returns [`UsageError::TotalOverflow`] when the sum overflows or exceeds
    /// the JSON safe-integer range.
    pub fn total_tokens(&self) -> Result<TokenCount, UsageError> {
        let total = [
            Some(self.input_tokens),
            Some(self.output_tokens),
            self.cache_read_tokens,
            self.cache_write_tokens,
        ]
        .into_iter()
        .flatten()
        .try_fold(0_u64, |total, value| total.checked_add(value.get()))
        .ok_or(UsageError::TotalOverflow)?;
        TokenCount::new(total)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawUsage {
    #[serde(rename = "inputTokens")]
    input: TokenCount,
    #[serde(rename = "outputTokens")]
    output: TokenCount,
    #[serde(default, rename = "cacheReadTokens")]
    cache_read: Option<TokenCount>,
    #[serde(default, rename = "cacheWriteTokens")]
    cache_write: Option<TokenCount>,
    #[serde(default, rename = "reasoningTokens")]
    reasoning: Option<TokenCount>,
}

impl<'de> Deserialize<'de> for Usage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawUsage::deserialize(deserializer)?;
        let mut usage = Self::new(raw.input, raw.output);
        usage.cache_read_tokens = raw.cache_read;
        usage.cache_write_tokens = raw.cache_write;
        if let Some(reasoning) = raw.reasoning {
            usage = usage
                .with_reasoning(reasoning)
                .map_err(serde::de::Error::custom)?;
        }
        usage.total_tokens().map_err(serde::de::Error::custom)?;
        Ok(usage)
    }
}

/// Error returned when validating token usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UsageError {
    /// A count exceeds JavaScript's safe integer range.
    #[error("token count exceeds the JSON safe-integer range")]
    UnsafeInteger,
    /// Reasoning tokens exceed output tokens.
    #[error("reasoning tokens must be a subset of output tokens")]
    ReasoningExceedsOutput,
    /// Summing usage counts overflowed or exceeded the safe range.
    #[error("total token count exceeds the supported range")]
    TotalOverflow,
}

/// A canonical, non-negative decimal amount encoded as text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DecimalAmount(String);

impl DecimalAmount {
    /// Returns the canonical decimal representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Error returned when parsing an exact decimal amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DecimalAmountParseError {
    /// The amount is not canonical non-negative decimal text.
    #[error("amount must use canonical non-negative decimal text")]
    InvalidFormat,
    /// The amount exceeds the supported precision or scale.
    #[error("amount exceeds the supported precision or scale")]
    TooPrecise,
}

impl FromStr for DecimalAmount {
    type Err = DecimalAmountParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (integer, fraction) = value
            .split_once('.')
            .map_or((value, None), |(left, right)| (left, Some(right)));
        if integer.is_empty()
            || !integer.bytes().all(|byte| byte.is_ascii_digit())
            || (integer.len() > 1 && integer.starts_with('0'))
        {
            return Err(DecimalAmountParseError::InvalidFormat);
        }
        if let Some(fraction) = fraction {
            if fraction.is_empty()
                || !fraction.bytes().all(|byte| byte.is_ascii_digit())
                || fraction.ends_with('0')
            {
                return Err(DecimalAmountParseError::InvalidFormat);
            }
            if fraction.len() > MAX_DECIMAL_SCALE {
                return Err(DecimalAmountParseError::TooPrecise);
            }
        }
        let digits = integer.len() + fraction.map_or(0, str::len);
        if digits > MAX_DECIMAL_DIGITS {
            return Err(DecimalAmountParseError::TooPrecise);
        }
        Ok(Self(value.to_owned()))
    }
}

impl fmt::Display for DecimalAmount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for DecimalAmount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for DecimalAmount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// A three-letter uppercase ISO-style currency code.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CurrencyCode(String);

impl CurrencyCode {
    /// Returns the three-letter code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Error returned when parsing a currency code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("currency must contain exactly three uppercase ASCII letters")]
pub struct CurrencyCodeParseError;

impl FromStr for CurrencyCode {
    type Err = CurrencyCodeParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_uppercase()) {
            Ok(Self(value.to_owned()))
        } else {
            Err(CurrencyCodeParseError)
        }
    }
}

impl fmt::Display for CurrencyCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for CurrencyCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CurrencyCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Unit used by exact currency amounts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostUnit {
    /// Amount is denominated in the currency's major unit, such as dollars.
    MajorCurrency,
}

/// An exact persisted monetary amount.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactCost {
    amount: DecimalAmount,
    currency: CurrencyCode,
    unit: CostUnit,
}

impl ExactCost {
    /// Creates a cost denominated in a currency's major unit.
    #[must_use]
    pub const fn new(amount: DecimalAmount, currency: CurrencyCode) -> Self {
        Self {
            amount,
            currency,
            unit: CostUnit::MajorCurrency,
        }
    }

    /// Returns the exact decimal amount.
    #[must_use]
    pub const fn amount(&self) -> &DecimalAmount {
        &self.amount
    }

    /// Returns the currency code.
    #[must_use]
    pub const fn currency(&self) -> &CurrencyCode {
        &self.currency
    }

    /// Returns the amount unit.
    #[must_use]
    pub const fn unit(&self) -> CostUnit {
        self.unit
    }
}
