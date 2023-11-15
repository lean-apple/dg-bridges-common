use codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use sp_core::H256;
use sp_runtime::{FixedU128, RuntimeDebug};

/// Minimal delivery fee factor.
pub const MINIMAL_DELIVERY_FEE_FACTOR: FixedU128 = FixedU128::from_u32(1);

/// XCM channel status provider that may report whether it is congested or not.
///
/// By channel we mean the physical channel that is used to deliver messages of one
/// of the bridge queues.
pub trait XcmChannelStatusProvider {
	/// Returns true if the channel is currently congested.
	fn is_congested() -> bool;
}

impl XcmChannelStatusProvider for () {
	fn is_congested() -> bool {
		false
	}
}

/// Current status of the bridge.
#[derive(Clone, Decode, Encode, Eq, PartialEq, TypeInfo, MaxEncodedLen, RuntimeDebug)]
pub struct BridgeState {
	/// Current delivery fee factor.
	pub delivery_fee_factor: FixedU128,
	/// Bridge congestion flag.
	pub is_congested: bool,
}

impl Default for BridgeState {
	fn default() -> BridgeState {
		BridgeState { delivery_fee_factor: MINIMAL_DELIVERY_FEE_FACTOR, is_congested: false }
	}
}

/// A minimized version of `pallet-xcm-bridge-hub-router::Call` that can be used without a runtime.
#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone, TypeInfo)]
#[allow(non_camel_case_types)]
pub enum XcmBridgeHubRouterCall {
	/// `pallet-xcm-bridge-hub-router::Call::report_bridge_status`
	#[codec(index = 0)]
	report_bridge_status { bridge_id: H256, is_congested: bool },
}