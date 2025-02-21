//! Fact topology type and helpers.
//!
//! Ported from https://github.com/starkware-libs/cairo-lang/blob/master/src/starkware/cairo/bootloaders/fact_topology.py

use std::collections::HashMap;

use cairo_vm::types::builtin_name::BuiltinName;
use cairo_vm::vm::runners::cairo_pie::{BuiltinAdditionalData, CairoPie, PublicMemoryPage};
use orchestrator_utils::ensure;

use super::error::FactError;

pub const GPS_FACT_TOPOLOGY: &str = "gps_fact_topology";

/// Flattened fact tree
#[derive(Debug, Clone)]
pub struct FactTopology {
    /// List of pairs (n_pages, n_nodes)
    pub tree_structure: Vec<usize>,
    /// Page sizes (pages are leaf nodes)
    pub page_sizes: Vec<usize>,
}

/// Returns the fact topology from the additional data of the output builtin.
pub fn get_fact_topology(cairo_pie: &CairoPie, output_size: usize) -> Result<FactTopology, FactError> {
    tracing::debug!(
        log_type = "FactTopology",
        category = "get_fact_topology",
        function_type = "get_fact_topology",
        "Starting get_fact_topology function"
    );
    if let Some(BuiltinAdditionalData::Output(additional_data)) = cairo_pie.additional_data.0.get(&BuiltinName::output)
    {
        tracing::trace!("Found Output additional data");
        let tree_structure = match additional_data.attributes.get(GPS_FACT_TOPOLOGY) {
            Some(tree_structure) => {
                tracing::debug!("GPS_FACT_TOPOLOGY found in additional data attributes");
                ensure!(!tree_structure.is_empty(), FactError::TreeStructureEmpty);
                ensure!(tree_structure.len() % 2 == 0, FactError::TreeStructureLenOdd);
                ensure!(tree_structure.len() <= 10, FactError::TreeStructureTooLarge);
                ensure!(tree_structure.iter().all(|&x| x < 2 << 30), FactError::TreeStructureInvalid);
                tracing::trace!("Tree structure validation passed");
                tree_structure.clone()
            }
            None => {
                tracing::warn!("GPS_FACT_TOPOLOGY not found in additional data attributes");
                ensure!(additional_data.pages.is_empty(), FactError::OutputPagesLenUnexpected);
                tracing::debug!("Using default tree structure");
                vec![1, 0]
            }
        };
        tracing::debug!("Retrieving page sizes");
        let page_sizes = get_page_sizes(&additional_data.pages, output_size)?;
        tracing::debug!(
            log_type = "FactTopology",
            category = "get_fact_topology",
            function_type = "get_fact_topology",
            "FactTopology successfully created"
        );
        Ok(FactTopology { tree_structure, page_sizes })
    } else {
        tracing::error!("Failed to get Output additional data");
        Err(FactError::OutputBuiltinNoAdditionalData)
    }
}

/// Returns the sizes of the program output pages, given the pages dictionary that appears
/// in the additional attributes of the output builtin.
pub fn get_page_sizes(pages: &HashMap<usize, PublicMemoryPage>, output_size: usize) -> Result<Vec<usize>, FactError> {
    tracing::debug!(
        log_type = "FactTopology",
        category = "get_page_sizes",
        function_type = "get_page_sizes",
        "Starting get_page_sizes function"
    );
    let mut pages_list: Vec<(usize, usize, usize)> =
        pages.iter().map(|(&id, page)| (id, page.start, page.size)).collect();
    pages_list.sort();
    tracing::debug!("FactTopology Sorted pages list: {:?}", pages_list);

    // The first page id is expected to be 1.
    let mut expected_page_id = 1;
    // We don't expect anything on its start value.
    let mut expected_page_start = None;

    let mut page_sizes = Vec::with_capacity(pages_list.len() + 1);
    // The size of page 0 is output_size if there are no other pages, or the start of page 1 otherwise.
    page_sizes.push(output_size);
    tracing::debug!("FactTopology Initialized page_sizes with output_size: {}", output_size);

    for (page_id, page_start, page_size) in pages_list {
        tracing::debug!("FactTopology Processing page: id={}, start={}, size={}", page_id, page_start, page_size);
        ensure!(page_id == expected_page_id, FactError::OutputPagesUnexpectedId(page_id, expected_page_id));

        if page_id == 1 {
            tracing::trace!("FactTopology Processing first page");
            ensure!(
                page_start > 0 && page_start < output_size,
                FactError::OutputPagesInvalidStart(page_id, page_start, output_size)
            );
            page_sizes[0] = page_start;
            tracing::debug!("FactTopology Updated page_sizes[0] to {}", page_start);
        } else {
            tracing::trace!("FactTopology Processing non-first page");
            ensure!(
                Some(page_start) == expected_page_start,
                FactError::OutputPagesUnexpectedStart(page_id, page_start, expected_page_start.unwrap_or_default(),) /* The unwrap here is fine as the assert is exactly for this reason */
            );
        }

        ensure!(
            page_size > 0 && page_size < output_size,
            FactError::OutputPagesInvalidSize(page_id, page_size, output_size)
        );
        expected_page_start = Some(page_start + page_size);
        expected_page_id += 1;

        page_sizes.push(page_size);
        tracing::trace!("FactTopology Added page_size {} to page_sizes", page_size);
    }

    tracing::debug!("FactTopology Final page_sizes: {:?}", page_sizes);

    ensure!(
        pages.is_empty() || expected_page_start == Some(output_size),
        FactError::OutputPagesUncoveredOutput(expected_page_start.unwrap_or_default(), output_size) /* The unwrap here is fine as the assert is exactly for this reason */
    );

    tracing::debug!(
        log_type = "FactTopology",
        category = "get_page_sizes",
        "FactTopology Successfully generated page sizes"
    );
    Ok(page_sizes)
}
