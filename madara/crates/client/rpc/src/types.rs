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
    // Continuation tokens track the offset among matching events, not the raw event index inside
    // the block. This matters when an API-level filter excludes some events from the DB batch.
    if events_infos.len() <= page_size {
        return None;
    }

    let last_block_number = events_infos.last()?.block_number;
    let number_of_events_in_last_block =
        events_infos.iter().rev().take_while(|event| event.block_number == last_block_number).count();

    if number_of_events_in_last_block < events_infos.len() {
        // The extra event spills into a later block, so resume from the count of matching events
        // already returned in that block.
        Some(ContinuationToken {
            block_number: last_block_number,
            event_n: number_of_events_in_last_block.saturating_sub(1) as u64,
        })
    } else {
        // All fetched events are still in the same block; keep the block number and advance by
        // the number of matching events returned for this page.
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

    #[rstest]
    #[case(&[0, 0, 0], 2, ContinuationToken { block_number: 0, event_n: 0 }, Some(ContinuationToken { block_number: 0, event_n: 2 }))]
    #[case(&[0, 0, 4], 2, ContinuationToken { block_number: 0, event_n: 0 }, Some(ContinuationToken { block_number: 4, event_n: 0 }))]
    #[case(&[4, 6, 6], 2, ContinuationToken { block_number: 0, event_n: 2 }, Some(ContinuationToken { block_number: 6, event_n: 1 }))]
    fn continuation_token_from_page_tracks_block_boundaries(
        #[case] block_numbers: &[u64],
        #[case] page_size: usize,
        #[case] previous_token: ContinuationToken,
        #[case] expected: Option<ContinuationToken>,
    ) {
        let events = block_numbers.iter().copied().map(dummy_event).collect::<Vec<_>>();

        let token = continuation_token_from_page(&events, page_size, &previous_token);
        assert_eq!(token, expected);
    }
}
