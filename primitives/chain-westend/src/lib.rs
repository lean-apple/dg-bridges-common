// Copyright (C) Parity Technologies (UK) Ltd.
// This file is part of Parity Bridges Common.

// Parity Bridges Common is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Parity Bridges Common is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Parity Bridges Common.  If not, see <http://www.gnu.org/licenses/>.

//! Primitives of the Westend chain.

#![warn(missing_docs)]
#![cfg_attr(not(feature = "std"), no_std)]

pub use bp_polkadot_core::*;
use sp_runtime::StateVersion;

use bp_header_chain::ChainWithGrandpa;
use bp_runtime::{decl_bridge_finality_runtime_apis, Chain, ChainId};
use frame_support::weights::Weight;

/// Westend Chain
pub struct Westend;

impl Chain for Westend {
	const ID: ChainId = *b"wend";

	type BlockNumber = BlockNumber;
	type Hash = Hash;
	type Hasher = Hasher;
	type Header = Header;

	type AccountId = AccountId;
	type Balance = Balance;
	type Nonce = Nonce;
	type Signature = Signature;

	const STATE_VERSION: StateVersion = StateVersion::V1;

	fn max_extrinsic_size() -> u32 {
		max_extrinsic_size()
	}

	fn max_extrinsic_weight() -> Weight {
		max_extrinsic_weight()
	}
}

impl ChainWithGrandpa for Westend {
	const WITH_CHAIN_GRANDPA_PALLET_NAME: &'static str = WITH_WESTEND_GRANDPA_PALLET_NAME;
	const MAX_AUTHORITIES_COUNT: u32 = MAX_AUTHORITIES_COUNT;
	const REASONABLE_HEADERS_IN_JUSTIFICATON_ANCESTRY: u32 =
		REASONABLE_HEADERS_IN_JUSTIFICATON_ANCESTRY;
	const MAX_HEADER_SIZE: u32 = MAX_HEADER_SIZE;
	const AVERAGE_HEADER_SIZE_IN_JUSTIFICATION: u32 = AVERAGE_HEADER_SIZE_IN_JUSTIFICATION;
}

// The SignedExtension used by Westend.
pub use bp_polkadot_core::CommonSignedExtension as SignedExtension;

/// Name of the parachains pallet in the Rococo runtime.
pub const PARAS_PALLET_NAME: &str = "Paras";

/// Name of the With-Westend GRANDPA pallet instance that is deployed at bridged chains.
pub const WITH_WESTEND_GRANDPA_PALLET_NAME: &str = "BridgeWestendGrandpa";

/// Maximal size of encoded `bp_parachains::ParaStoredHeaderData` structure among all Westend
/// parachains.
///
/// It includes the block number and state root, so it shall be near 40 bytes, but let's have some
/// reserve.
pub const MAX_NESTED_PARACHAIN_HEAD_DATA_SIZE: u32 = 128;

decl_bridge_finality_runtime_apis!(westend, grandpa);
