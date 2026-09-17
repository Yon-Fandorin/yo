use super::model::{
    AggregateValue, AggregatedUsage, DerivedTokenValue, InputAmplification, UsageAnalysis,
    UsageReceipt, UsageValue,
};

pub(super) fn aggregate(receipts: &[UsageReceipt]) -> Result<AggregatedUsage, String> {
    Ok(AggregatedUsage {
        input_tokens: aggregate_field(receipts.iter().map(|receipt| receipt.usage.input_tokens))?,
        output_tokens: aggregate_field(receipts.iter().map(|receipt| receipt.usage.output_tokens))?,
        total_tokens: aggregate_field(receipts.iter().map(|receipt| receipt.usage.total_tokens))?,
        reasoning_tokens: aggregate_field(
            receipts
                .iter()
                .map(|receipt| receipt.usage.reasoning_tokens),
        )?,
        cache_read_input_tokens: aggregate_field(
            receipts
                .iter()
                .map(|receipt| receipt.usage.cache_read_input_tokens),
        )?,
        cache_write_input_tokens: aggregate_field(
            receipts
                .iter()
                .map(|receipt| receipt.usage.cache_write_input_tokens),
        )?,
    })
}

pub(super) fn aggregate_field(
    values: impl IntoIterator<Item = UsageValue>,
) -> Result<AggregateValue, String> {
    let mut tokens = 0_u64;
    let mut reported = 0_usize;
    let mut absent = 0_usize;
    let mut unsupported = 0_usize;
    for value in values {
        match value {
            UsageValue::Reported { tokens: value } => {
                tokens = tokens
                    .checked_add(value)
                    .ok_or_else(|| "exact delivery usage aggregate overflowed u64".to_owned())?;
                reported += 1;
            },
            UsageValue::Absent => absent += 1,
            UsageValue::Unsupported => unsupported += 1,
        }
    }
    let total = reported + absent + unsupported;
    Ok(if reported == total && total > 0 {
        AggregateValue::Reported { tokens }
    } else if reported > 0 {
        AggregateValue::Partial {
            tokens,
            reported_receipts: reported,
            total_receipts: total,
        }
    } else {
        AggregateValue::Unavailable {
            absent_receipts: absent,
            unsupported_receipts: unsupported,
            total_receipts: total,
        }
    })
}

pub(super) fn analyze(usage: &AggregatedUsage, packet_managed_tokens: usize) -> UsageAnalysis {
    let uncached_input_tokens = match (&usage.input_tokens, &usage.cache_read_input_tokens) {
        (
            AggregateValue::Reported { tokens: input },
            AggregateValue::Reported { tokens: cache_read },
        ) if cache_read <= input => DerivedTokenValue::Reported {
            tokens: input - cache_read,
            derivation: "input_tokens-cache_read_input_tokens",
        },
        (
            AggregateValue::Reported { tokens: input },
            AggregateValue::Reported { tokens: cache_read },
        ) if cache_read > input => DerivedTokenValue::Unavailable {
            reason: "cache_read_exceeds_input",
        },
        _ => DerivedTokenValue::Unavailable {
            reason: "incomplete_input_or_cache_read_coverage",
        },
    };
    let input_amplification = match (&usage.input_tokens, packet_managed_tokens) {
        (_, 0) => InputAmplification::Unavailable {
            reason: "packet_managed_tokens_zero",
        },
        (AggregateValue::Reported { tokens }, packet_managed_tokens) => {
            let ratio =
                ((*tokens as f64 / packet_managed_tokens as f64) * 1_000.0).round() / 1_000.0;
            InputAmplification::Reported {
                ratio,
                provider_input_tokens: *tokens,
                packet_managed_tokens,
            }
        },
        _ => InputAmplification::Unavailable {
            reason: "incomplete_input_coverage",
        },
    };
    UsageAnalysis {
        packet_managed_tokens,
        uncached_input_tokens,
        input_amplification,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AggregateValue, AggregatedUsage, DerivedTokenValue, InputAmplification, UsageAnalysis,
        UsageValue, aggregate_field, analyze,
    };

    // 모든 receipt가 값을 보고하면 합계는 완전한 reported 값이고, 하나라도 absent면
    // 알려진 합계를 버리지 않되 coverage를 partial로 명시합니다.
    #[test]
    fn aggregate_preserves_reported_and_partial_coverage() {
        assert_eq!(
            aggregate_field([
                UsageValue::Reported { tokens: 3 },
                UsageValue::Reported { tokens: 4 },
            ])
            .unwrap(),
            AggregateValue::Reported { tokens: 7 }
        );
        assert_eq!(
            aggregate_field([UsageValue::Reported { tokens: 3 }, UsageValue::Absent]).unwrap(),
            AggregateValue::Partial {
                tokens: 3,
                reported_receipts: 1,
                total_receipts: 2,
            }
        );
    }

    // receipt 자체가 없거나 필드가 전부 비보고 상태이면 0 token을 꾸며내지 않고
    // absent와 unsupported 개수를 포함한 unavailable 값으로 남깁니다.
    #[test]
    fn aggregate_keeps_unavailable_distinct_from_reported_zero() {
        assert_eq!(
            aggregate_field([]).unwrap(),
            AggregateValue::Unavailable {
                absent_receipts: 0,
                unsupported_receipts: 0,
                total_receipts: 0,
            }
        );
        assert_eq!(
            aggregate_field([UsageValue::Absent, UsageValue::Unsupported]).unwrap(),
            AggregateValue::Unavailable {
                absent_receipts: 1,
                unsupported_receipts: 1,
                total_receipts: 2,
            }
        );
        assert_eq!(
            aggregate_field([UsageValue::Reported { tokens: 0 }]).unwrap(),
            AggregateValue::Reported { tokens: 0 }
        );
    }

    // packet 크기와 완전하게 보고된 provider/cache 입력에서만 비캐시 입력과 읽기 쉬운
    // 3자리 증폭률을 파생하고, cache token을 Provider 입력에 다시 더하지 않습니다.
    #[test]
    fn analysis_separates_uncached_input_and_packet_amplification() {
        let usage = complete_usage(990_347, 585_728);
        assert_eq!(
            analyze(&usage, 43_276),
            UsageAnalysis {
                packet_managed_tokens: 43_276,
                uncached_input_tokens: DerivedTokenValue::Reported {
                    tokens: 404_619,
                    derivation: "input_tokens-cache_read_input_tokens",
                },
                input_amplification: InputAmplification::Reported {
                    ratio: 22.884,
                    provider_input_tokens: 990_347,
                    packet_managed_tokens: 43_276,
                },
            }
        );
    }

    // 불완전한 coverage, 0-byte packet, 또는 input보다 큰 cache 보고는 0이나 음수를
    // 꾸며내지 않고 각 파생치의 구체적인 unavailable 이유로 남깁니다.
    #[test]
    fn analysis_fails_closed_when_inputs_do_not_support_a_derivation() {
        let mut partial = complete_usage(10, 4);
        partial.input_tokens = AggregateValue::Partial {
            tokens: 10,
            reported_receipts: 1,
            total_receipts: 2,
        };
        assert_eq!(
            analyze(&partial, 5),
            UsageAnalysis {
                packet_managed_tokens: 5,
                uncached_input_tokens: DerivedTokenValue::Unavailable {
                    reason: "incomplete_input_or_cache_read_coverage",
                },
                input_amplification: InputAmplification::Unavailable {
                    reason: "incomplete_input_coverage",
                },
            }
        );

        let invalid_cache = complete_usage(3, 4);
        assert_eq!(
            analyze(&invalid_cache, 0),
            UsageAnalysis {
                packet_managed_tokens: 0,
                uncached_input_tokens: DerivedTokenValue::Unavailable {
                    reason: "cache_read_exceeds_input",
                },
                input_amplification: InputAmplification::Unavailable {
                    reason: "packet_managed_tokens_zero",
                },
            }
        );
    }

    fn complete_usage(input_tokens: u64, cache_read_input_tokens: u64) -> AggregatedUsage {
        AggregatedUsage {
            input_tokens: AggregateValue::Reported {
                tokens: input_tokens,
            },
            output_tokens: AggregateValue::Reported { tokens: 0 },
            total_tokens: AggregateValue::Reported {
                tokens: input_tokens,
            },
            reasoning_tokens: AggregateValue::Reported { tokens: 0 },
            cache_read_input_tokens: AggregateValue::Reported {
                tokens: cache_read_input_tokens,
            },
            cache_write_input_tokens: AggregateValue::Reported { tokens: 0 },
        }
    }
}
