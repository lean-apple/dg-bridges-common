// Copyright 2022 Parity Technologies (UK) Ltd.
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

//! Types used to connect to the BridgeHub-Westend-Substrate parachain.

use bp_bridge_hub_westend::AVERAGE_BLOCK_INTERVAL;
use bp_polkadot_core::SuffixedCommonSignedExtensionExt;
use codec::Encode;
use relay_substrate_client::{
	Chain, ChainWithBalances, ChainWithMessages, ChainWithTransactions, ChainWithUtilityPallet,
	Error as SubstrateError, MockedRuntimeUtilityPallet, SignParam, UnderlyingChainProvider,
	UnsignedTransaction,
};
use sp_core::{storage::StorageKey, Pair};
use sp_runtime::{generic::SignedPayload, traits::IdentifyAccount};
use std::time::Duration;

/// Re-export runtime wrapper
pub mod runtime_wrapper;
pub use runtime_wrapper as runtime;

/// Westend chain definition
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeHubWestend;

impl UnderlyingChainProvider for BridgeHubWestend {
	type Chain = bp_bridge_hub_westend::BridgeHubWestend;
}

impl Chain for BridgeHubWestend {
	const NAME: &'static str = "BridgeHubWestend";
	const BEST_FINALIZED_HEADER_ID_METHOD: &'static str =
		bp_bridge_hub_westend::BEST_FINALIZED_BRIDGE_HUB_WESTEND_HEADER_METHOD;
	const AVERAGE_BLOCK_INTERVAL: Duration = AVERAGE_BLOCK_INTERVAL;

	type SignedBlock = bp_bridge_hub_westend::SignedBlock;
	type Call = runtime::Call;
}

impl ChainWithBalances for BridgeHubWestend {
	fn account_info_storage_key(account_id: &Self::AccountId) -> StorageKey {
		bp_bridge_hub_westend::AccountInfoStorageMapKeyProvider::final_key(account_id)
	}
}

impl ChainWithUtilityPallet for BridgeHubWestend {
	type UtilityPallet = MockedRuntimeUtilityPallet<runtime::Call>;
}

impl ChainWithTransactions for BridgeHubWestend {
	type AccountKeyPair = sp_core::sr25519::Pair;
	type SignedTransaction = runtime::UncheckedExtrinsic;

	fn sign_transaction(
		param: SignParam<Self>,
		unsigned: UnsignedTransaction<Self>,
	) -> Result<Self::SignedTransaction, SubstrateError> {
		let raw_payload = SignedPayload::new(
			unsigned.call,
			runtime::SignedExtension::from_params(
				param.spec_version,
				param.transaction_version,
				unsigned.era,
				param.genesis_hash,
				unsigned.nonce,
				unsigned.tip,
				(((), ()), ((), ())),
			),
		)?;

		let signature = raw_payload.using_encoded(|payload| param.signer.sign(payload));
		let signer: sp_runtime::MultiSigner = param.signer.public().into();
		let (call, extra, _) = raw_payload.deconstruct();

		Ok(runtime::UncheckedExtrinsic::new_signed(
			call,
			signer.into_account().into(),
			signature.into(),
			extra,
		))
	}

	fn is_signed(tx: &Self::SignedTransaction) -> bool {
		tx.signature.is_some()
	}

	fn is_signed_by(signer: &Self::AccountKeyPair, tx: &Self::SignedTransaction) -> bool {
		tx.signature
			.as_ref()
			.map(|(address, _, _)| {
				*address == bp_bridge_hub_westend::Address::Id(signer.public().into())
			})
			.unwrap_or(false)
	}

	fn parse_transaction(tx: Self::SignedTransaction) -> Option<UnsignedTransaction<Self>> {
		let extra = &tx.signature.as_ref()?.2;
		Some(UnsignedTransaction::new(tx.function, extra.nonce()).tip(extra.tip()))
	}
}

impl ChainWithMessages for BridgeHubWestend {
	const WITH_CHAIN_RELAYERS_PALLET_NAME: Option<&'static str> =
		Some(bp_bridge_hub_westend::WITH_BRIDGE_HUB_WESTEND_RELAYERS_PALLET_NAME);

	const TO_CHAIN_MESSAGE_DETAILS_METHOD: &'static str =
		bp_bridge_hub_westend::TO_BRIDGE_HUB_WESTEND_MESSAGE_DETAILS_METHOD;
	const FROM_CHAIN_MESSAGE_DETAILS_METHOD: &'static str =
		bp_bridge_hub_westend::FROM_BRIDGE_HUB_WESTEND_MESSAGE_DETAILS_METHOD;
}

#[cfg(test)]
mod tests {
	use super::*;
	use relay_substrate_client::TransactionEra;

	#[test]
	fn parse_transaction_works() {
		let unsigned = UnsignedTransaction {
			call: runtime::Call::System(runtime::SystemCall::remark(b"Hello world!".to_vec()))
				.into(),
			nonce: 777,
			tip: 888,
			era: TransactionEra::immortal(),
		};
		let signed_transaction = BridgeHubWestend::sign_transaction(
			SignParam {
				spec_version: 42,
				transaction_version: 50000,
				genesis_hash: [42u8; 32].into(),
				signer: sp_core::sr25519::Pair::from_seed_slice(&[1u8; 32]).unwrap(),
			},
			unsigned.clone(),
		)
		.unwrap();
		let parsed_transaction = BridgeHubWestend::parse_transaction(signed_transaction).unwrap();
		assert_eq!(parsed_transaction, unsigned);
	}
}
