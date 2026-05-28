use decibel_hotindex_core::{reverse_ts_us, DecibelEventType, Network};

const SEP: u8 = 0;

pub fn be_u64(value: u64) -> [u8; 8] {
    value.to_be_bytes()
}

pub fn be_u32(value: u32) -> [u8; 4] {
    value.to_be_bytes()
}

pub fn reverse_ts_key(ts_us: u64) -> [u8; 8] {
    be_u64(reverse_ts_us(ts_us))
}

pub fn tx_by_version(version: u64) -> Vec<u8> {
    be_u64(version).to_vec()
}

pub fn raw_event_by_version_idx(version: u64, event_idx: u32) -> Vec<u8> {
    let mut key = Vec::with_capacity(12);
    key.extend_from_slice(&be_u64(version));
    key.extend_from_slice(&be_u32(event_idx));
    key
}

pub fn event_by_type_version(
    event_type: &DecibelEventType,
    version: u64,
    event_idx: u32,
) -> Vec<u8> {
    join_segments(&[
        event_type.as_key().as_bytes(),
        &be_u64(version),
        &be_u32(event_idx),
    ])
}

pub fn fills_by_market_time(
    market_id: &str,
    timestamp_us: u64,
    version: u64,
    fill_id: &str,
) -> Vec<u8> {
    join_segments(&[
        market_id.as_bytes(),
        &reverse_ts_key(timestamp_us),
        &be_u64(version),
        fill_id.as_bytes(),
    ])
}

pub fn fills_by_market_prefix(market_id: &str) -> Vec<u8> {
    prefix_segments(&[market_id.as_bytes()])
}

pub fn fills_by_account_time(
    account: &str,
    timestamp_us: u64,
    market_id: &str,
    fill_id: &str,
) -> Vec<u8> {
    join_segments(&[
        account.as_bytes(),
        &reverse_ts_key(timestamp_us),
        market_id.as_bytes(),
        fill_id.as_bytes(),
    ])
}

pub fn fills_by_account_prefix(account: &str) -> Vec<u8> {
    prefix_segments(&[account.as_bytes()])
}

pub fn order_by_id(order_id: &str) -> Vec<u8> {
    order_id.as_bytes().to_vec()
}

pub fn positions_by_account_market(account: &str, market_id: &str) -> Vec<u8> {
    join_segments(&[account.as_bytes(), market_id.as_bytes()])
}

pub fn positions_by_account_prefix(account: &str) -> Vec<u8> {
    prefix_segments(&[account.as_bytes()])
}

pub fn builder_code_fills(
    builder_addr: &str,
    timestamp_us: u64,
    market_id: &str,
    fill_id: &str,
) -> Vec<u8> {
    join_segments(&[
        builder_addr.as_bytes(),
        &reverse_ts_key(timestamp_us),
        market_id.as_bytes(),
        fill_id.as_bytes(),
    ])
}

pub fn builder_code_fills_prefix(builder_addr: &str) -> Vec<u8> {
    prefix_segments(&[builder_addr.as_bytes()])
}

pub fn market_activity(
    market_id: &str,
    timestamp_us: u64,
    activity_type: &str,
    version: u64,
    event_idx: u32,
) -> Vec<u8> {
    join_segments(&[
        market_id.as_bytes(),
        &reverse_ts_key(timestamp_us),
        activity_type.as_bytes(),
        &be_u64(version),
        &be_u32(event_idx),
    ])
}

pub fn market_activity_prefix(market_id: &str) -> Vec<u8> {
    prefix_segments(&[market_id.as_bytes()])
}

pub fn ingest_checkpoint(network: Network, package_address: &str) -> Vec<u8> {
    join_segments(&[network.as_str().as_bytes(), package_address.as_bytes()])
}

pub fn checksum_logical_cf(cf_name: &str) -> Vec<u8> {
    cf_name.as_bytes().to_vec()
}

fn join_segments(segments: &[&[u8]]) -> Vec<u8> {
    let len = segments.iter().map(|segment| segment.len() + 1).sum();
    let mut key = Vec::with_capacity(len);
    for (idx, segment) in segments.iter().enumerate() {
        if idx > 0 {
            key.push(SEP);
        }
        key.extend_from_slice(segment);
    }
    key
}

fn prefix_segments(segments: &[&[u8]]) -> Vec<u8> {
    let mut key = join_segments(segments);
    key.push(SEP);
    key
}

#[cfg(test)]
mod tests {
    use super::{be_u64, fills_by_market_time, raw_event_by_version_idx};

    #[test]
    fn big_endian_orders_numeric_values() {
        assert!(be_u64(1) < be_u64(2));
        assert!(be_u64(255) < be_u64(256));
    }

    #[test]
    fn raw_event_key_orders_by_version_then_event_idx() {
        assert!(raw_event_by_version_idx(1, 99) < raw_event_by_version_idx(2, 0));
        assert!(raw_event_by_version_idx(2, 0) < raw_event_by_version_idx(2, 1));
    }

    #[test]
    fn market_fill_key_orders_recent_first_within_market() {
        let newer = fills_by_market_time("BTC-PERP", 200, 1, "fill-b");
        let older = fills_by_market_time("BTC-PERP", 100, 1, "fill-a");
        assert!(newer < older);
    }
}
