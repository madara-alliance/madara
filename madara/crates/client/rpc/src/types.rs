use std::fmt;
use std::num::ParseIntError;

use mp_block::EventWithInfo;

#[derive(PartialEq, Eq, Debug, Default)]
pub struct ContinuationToken {
    pub block_number: u64,
    pub event_n: u64,
}

#[derive(PartialEq, Eq, Debug)]
pub enum ParseTokenError {
    WrongToken,
    ParseFailed(ParseIntError),
}

impl fmt::Display for ContinuationToken {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}-{}", self.block_number, self.event_n)
    }
}

impl ContinuationToken {
    pub fn parse(token: String) -> Result<Self, ParseTokenError> {
        let arr: Vec<&str> = token.split('-').collect();
        if arr.len() != 2 {
            return Err(ParseTokenError::WrongToken);
        }
        let block_n = arr[0].parse::<u64>().map_err(ParseTokenError::ParseFailed)?;
        let event_n = arr[1].parse::<u64>().map_err(ParseTokenError::ParseFailed)?;

        Ok(ContinuationToken { block_number: block_n, event_n })
    }
}

pub fn continuation_token_from_page(
    events_infos: &[EventWithInfo],
    page_size: usize,
    previous_token: &ContinuationToken,
) -> Option<ContinuationToken> {
    if events_infos.len() <= page_size {
        return None;
    }

    let last_block_number = events_infos.last()?.block_number;
    let number_of_events_in_last_block =
        events_infos.iter().rev().take_while(|event| event.block_number == last_block_number).count();

    if number_of_events_in_last_block < events_infos.len() {
        Some(ContinuationToken {
            block_number: last_block_number,
            event_n: number_of_events_in_last_block.saturating_sub(1) as u64,
        })
    } else {
        Some(ContinuationToken {
            block_number: previous_token.block_number,
            event_n: previous_token.event_n + page_size as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use starknet_types_core::felt::Felt;

    use rstest::rstest;

    use crate::types::*;

    #[rstest]
    #[case(0, 0, "0-0")]
    #[case(1, 4, "1-4")]
    #[case(2, 4, "2-4")]
    #[case(0, 4, "0-4")]
    fn to_string_works(#[case] block_number: u64, #[case] event_n: u64, #[case] expected: String) {
        let token = ContinuationToken { block_number, event_n };
        assert_eq!(expected, token.to_string())
    }

    #[rstest]
    #[case("0-0", 0, 0)]
    #[case("1-4", 1, 4)]
    #[case("2-4", 2, 4)]
    fn parse_works(#[case] string_token: String, #[case] block_number: u64, #[case] event_n: u64) {
        let expected = ContinuationToken { block_number, event_n };
        assert_eq!(expected, ContinuationToken::parse(string_token).unwrap());
    }

    #[rstest]
    #[case("100")]
    #[case("0,")]
    #[case("0,0,0")]
    fn parse_should_fail(#[case] string_token: String) {
        let result = ContinuationToken::parse(string_token);
        assert!(result.is_err());
    }

    #[rstest]
    #[case("2y,4")]
    #[case("30,255g")]
    #[case("1,1,")]
    fn parse_u64_should_fail(#[case] string_token: String) {
        let result = ContinuationToken::parse(string_token);
        assert!(result.is_err());
    }

    fn dummy_event(block_number: u64) -> EventWithInfo {
        EventWithInfo {
            event: mp_receipt::Event { from_address: Felt::ZERO, keys: vec![], data: vec![] },
            block_number,
            block_hash: None,
            transaction_hash: Felt::ZERO,
            transaction_index: 0,
            event_index_in_block: 0,
            in_preconfirmed: false,
        }
    }

    #[test]
    fn continuation_token_stays_on_same_block_when_extra_event_is_in_same_block() {
        let events = vec![dummy_event(0), dummy_event(0), dummy_event(0)];
        let token = continuation_token_from_page(&events, 2, &ContinuationToken { block_number: 0, event_n: 0 });

        assert_eq!(token, Some(ContinuationToken { block_number: 0, event_n: 2 }));
    }

    #[test]
    fn continuation_token_moves_to_new_block_when_extra_event_is_first_match_there() {
        let events = vec![dummy_event(0), dummy_event(0), dummy_event(4)];
        let token = continuation_token_from_page(&events, 2, &ContinuationToken { block_number: 0, event_n: 0 });

        assert_eq!(token, Some(ContinuationToken { block_number: 4, event_n: 0 }));
    }

    #[test]
    fn continuation_token_tracks_offset_inside_new_block() {
        let events = vec![dummy_event(4), dummy_event(6), dummy_event(6)];
        let token = continuation_token_from_page(&events, 2, &ContinuationToken { block_number: 0, event_n: 2 });

        assert_eq!(token, Some(ContinuationToken { block_number: 6, event_n: 1 }));
    }
}
