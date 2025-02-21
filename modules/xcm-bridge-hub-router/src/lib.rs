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

//! Pallet that may be used instead of `SovereignPaidRemoteExporter` in the XCM router
//! configuration. The main thing that the pallet offers is the dynamic message fee,
//! that is computed based on the bridge queues state. It starts exponentially increasing
//! if the queue between this chain and the sibling/child bridge hub is congested.
//!
//! All other bridge hub queues offer some backpressure mechanisms. So if at least one
//! of all queues is congested, it will eventually lead to the growth of the queue at
//! this chain.
//!
//! **A note on terminology**: when we mention the bridge hub here, we mean the chain that
//! has the messages pallet deployed (`pallet-bridge-grandpa`, `pallet-bridge-messages`,
//! `pallet-xcm-bridge-hub`, ...). It may be the system bridge hub parachain or any other
//! chain.

#![cfg_attr(not(feature = "std"), no_std)]

use bp_xcm_bridge_hub::LocalXcmChannelManager;
use codec::Encode;
use frame_support::traits::Get;
use sp_runtime::{FixedPointNumber, FixedU128, Saturating};
use xcm::prelude::*;
use xcm_builder::{ExporterFor, SovereignPaidRemoteExporter};

pub use pallet::*;
pub use weights::WeightInfo;

pub mod benchmarking;
pub mod weights;

mod mock;

/// Minimal delivery fee factor.
pub const MINIMAL_DELIVERY_FEE_FACTOR: FixedU128 = FixedU128::from_u32(1);

/// The factor that is used to increase current message fee factor when bridge experiencing
/// some lags.
const EXPONENTIAL_FEE_BASE: FixedU128 = FixedU128::from_rational(105, 100); // 1.05
/// The factor that is used to increase current message fee factor for every sent kilobyte.
const MESSAGE_SIZE_FEE_BASE: FixedU128 = FixedU128::from_rational(1, 1000); // 0.001

/// Maximal size of the XCM message that may be sent over bridge.
///
/// This should be less than the maximal size, allowed by the messages pallet, because
/// the message itself is wrapped in other structs and is double encoded.
pub const HARD_MESSAGE_SIZE_LIMIT: u32 = 32 * 1024;

/// The target that will be used when publishing logs related to this pallet.
///
/// This doesn't match the pattern used by other bridge pallets (`runtime::bridge-*`). But this
/// pallet has significant differences with those pallets. The main one is that is intended to
/// be deployed at sending chains. Other bridge pallets are likely to be deployed at the separate
/// bridge hub parachain.
pub const LOG_TARGET: &str = "xcm::bridge-hub-router";

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;

	#[pallet::config]
	pub trait Config<I: 'static = ()>: frame_system::Config {
		/// Benchmarks results from runtime we're plugged into.
		type WeightInfo: WeightInfo;

		/// Universal location of this runtime.
		type UniversalLocation: Get<InteriorMultiLocation>;
		/// Relative location of the sibling bridge hub.
		type SiblingBridgeHubLocation: Get<MultiLocation>;
		/// The bridged network that this config is for if specified.
		/// Also used for filtering `Bridges` by `BridgedNetworkId`.
		/// If not specified, allows all networks pass through.
		type BridgedNetworkId: Get<NetworkId>;
		/// Configuration for supported **bridged networks/locations** with **bridge location** and
		/// **possible fee**. Allows to externalize better control over allowed **bridged
		/// networks/locations**.
		type Bridges: ExporterFor;

		/// Actual message sender (`HRMP` or `DMP`) to the sibling bridge hub location.
		type ToBridgeHubSender: SendXcm;
		/// Local XCM channel manager.
		type LocalXcmChannelManager: LocalXcmChannelManager;

		/// Additional fee that is paid for every byte of the outbound message.
		type ByteFee: Get<u128>;
		/// Asset that is used to paid bridge fee.
		type FeeAsset: Get<AssetId>;
	}

	#[pallet::pallet]
	pub struct Pallet<T, I = ()>(PhantomData<(T, I)>);

	#[pallet::hooks]
	impl<T: Config<I>, I: 'static> Hooks<BlockNumberFor<T>> for Pallet<T, I> {
		fn on_initialize(_n: BlockNumberFor<T>) -> Weight {
			// if XCM channel is still congested, we don't change anything
			if T::LocalXcmChannelManager::is_congested(&T::SiblingBridgeHubLocation::get()) {
				return T::WeightInfo::on_initialize_when_congested()
			}

			// if we can't decrease the delivery fee factor anymore, we don't change anything
			let mut delivery_fee_factor = Self::delivery_fee_factor();
			if delivery_fee_factor == MINIMAL_DELIVERY_FEE_FACTOR {
				return T::WeightInfo::on_initialize_when_congested()
			}

			let previous_factor = delivery_fee_factor;
			delivery_fee_factor =
				MINIMAL_DELIVERY_FEE_FACTOR.max(delivery_fee_factor / EXPONENTIAL_FEE_BASE);
			log::info!(
				target: LOG_TARGET,
				"Bridge channel is uncongested. Decreased fee factor from {} to {}",
				previous_factor,
				delivery_fee_factor,
			);

			DeliveryFeeFactor::<T, I>::put(delivery_fee_factor);
			T::WeightInfo::on_initialize_when_non_congested()
		}
	}

	/// Initialization value for the delivery fee factor.
	#[pallet::type_value]
	pub fn InitialFactor() -> FixedU128 {
		MINIMAL_DELIVERY_FEE_FACTOR
	}

	/// The number to multiply the base delivery fee by.
	///
	/// This factor is shared by all bridges, served by this pallet. For example, if this
	/// chain (`Config::UniversalLocation`) opens two bridges (
	/// `X2(GlobalConsensus(Config::BridgedNetworkId::get()), Parachain(1000))` and
	/// `X2(GlobalConsensus(Config::BridgedNetworkId::get()), Parachain(2000))`), then they
	/// both will be sharing the same fee factor. This is because both bridges are sharing
	/// the same local XCM channel with the child/sibling bridge hub, which we are using
	/// to detect congestion:
	///
	/// ```nocompile
	///  ThisChain --- Local XCM chanel --> Sibling Bridge Hub ------
	///                                            |                   |
	///                                            |                   |
	///                                            |                   |
	///                                          Lane1               Lane2
	///                                            |                   |
	///                                            |                   |
	///                                            |                   |
	///                                           \ /                  |
	///  Parachain1  <-- Local XCM channel --- Remote Bridge Hub <------
	///                                            |
	///                                            |
	///  Parachain1  <-- Local XCM channel ---------
	/// ```
	///
	/// If at least one of other channels is congested, the local XCM channel with sibling
	/// bridge hub eventually becomes congested too. And we have no means to detect - which
	/// bridge exactly causes the congestion. So the best solution here is not to make
	/// any differences between all bridges, started by this chain.
	#[pallet::storage]
	#[pallet::getter(fn delivery_fee_factor)]
	pub type DeliveryFeeFactor<T: Config<I>, I: 'static = ()> =
		StorageValue<_, FixedU128, ValueQuery, InitialFactor>;

	impl<T: Config<I>, I: 'static> Pallet<T, I> {
		/// Called when new message is sent (queued to local outbound XCM queue) over the bridge.
		pub(crate) fn on_message_sent_to_bridge(message_size: u32) {
			// if outbound channel is not congested, do nothing
			if !T::LocalXcmChannelManager::is_congested(&T::SiblingBridgeHubLocation::get()) {
				return
			}

			// ok - we need to increase the fee factor, let's do that
			let message_size_factor = FixedU128::from_u32(message_size.saturating_div(1024))
				.saturating_mul(MESSAGE_SIZE_FEE_BASE);
			let total_factor = EXPONENTIAL_FEE_BASE.saturating_add(message_size_factor);
			DeliveryFeeFactor::<T, I>::mutate(|f| {
				let previous_factor = *f;
				*f = f.saturating_mul(total_factor);
				log::info!(
					target: LOG_TARGET,
					"Bridge channel is congested. Increased fee factor from {} to {}",
					previous_factor,
					f,
				);
				*f
			});
		}
	}
}

/// We'll be using `SovereignPaidRemoteExporter` to send remote messages over the sibling/child
/// bridge hub.
type ViaBridgeHubExporter<T, I> = SovereignPaidRemoteExporter<
	Pallet<T, I>,
	<T as Config<I>>::ToBridgeHubSender,
	<T as Config<I>>::UniversalLocation,
>;

// This pallet acts as the `ExporterFor` for the `SovereignPaidRemoteExporter` to compute
// message fee using fee factor.
impl<T: Config<I>, I: 'static> ExporterFor for Pallet<T, I> {
	fn exporter_for(
		network: &NetworkId,
		remote_location: &InteriorMultiLocation,
		message: &Xcm<()>,
	) -> Option<(MultiLocation, Option<MultiAsset>)> {
		// ensure that the message is sent to the expected bridged network (if specified).
		if *network != T::BridgedNetworkId::get() {
			log::trace!(
				target: LOG_TARGET,
				"Router with bridged_network_id {:?} does not support bridging to network {:?}!",
				T::BridgedNetworkId::get(),
				network,
			);
			return None
		}

		// ensure that the message is sent to the expected bridged network and location.
		let Some((bridge_hub_location, maybe_payment)) =
			T::Bridges::exporter_for(network, remote_location, message)
		else {
			log::trace!(
				target: LOG_TARGET,
				"Router with bridged_network_id {:?} does not support bridging to network {:?} \
				and remote_location {:?}!",
				T::BridgedNetworkId::get(),
				network,
				remote_location,
			);
			return None
		};

		// ensure that the bridge hub location is the expected one
		if bridge_hub_location != T::SiblingBridgeHubLocation::get() {
			log::trace!(
				target: LOG_TARGET,
				"Router with bridged_network_id {:?} is configured to use different bridge hub \
				router {:?}. Expected: {:?}",
				T::BridgedNetworkId::get(),
				bridge_hub_location,
				T::SiblingBridgeHubLocation::get(),
			);
			return None
		}

		// take `base_fee` from `T::Brides`, but it has to be the same `T::FeeAsset`
		let base_fee = match maybe_payment {
			Some(payment) => match payment {
				MultiAsset { fun: Fungible(amount), id } if id.eq(&T::FeeAsset::get()) => amount,
				invalid_asset => {
					log::error!(
						target: LOG_TARGET,
						"Router with bridged_network_id {:?} is configured for `T::FeeAsset` {:?} \
						which is not compatible with {:?} for bridge_hub_location: {:?} for bridging to {:?}/{:?}!",
						T::BridgedNetworkId::get(),
						T::FeeAsset::get(),
						invalid_asset,
						bridge_hub_location,
						network,
						remote_location,
					);
					return None
				},
			},
			None => 0,
		};

		// compute fee amount. Keep in mind that this is only the bridge fee. The fee for sending
		// message from this chain to child/sibling bridge hub is determined by the
		// `Config::ToBridgeHubSender`
		let message_size = message.encoded_size();
		let message_fee = (message_size as u128).saturating_mul(T::ByteFee::get());
		let fee_sum = base_fee.saturating_add(message_fee);
		let fee_factor = Self::delivery_fee_factor();
		let fee = fee_factor.saturating_mul_int(fee_sum);
		let fee = if fee > 0 { Some((T::FeeAsset::get(), fee).into()) } else { None };

		log::info!(
			target: LOG_TARGET,
			"Going to send message ({} bytes) over bridge. Computed bridge fee {:?} using fee factor {}",
			message_size,
			fee,
			fee_factor,
		);

		Some((bridge_hub_location, fee))
	}
}

// This pallet acts as the `SendXcm` to the sibling/child bridge hub instead of regular
// XCMP/DMP transport. This allows injecting dynamic message fees into XCM programs that
// are going to the bridged network.
impl<T: Config<I>, I: 'static> SendXcm for Pallet<T, I> {
	type Ticket = (u32, <T::ToBridgeHubSender as SendXcm>::Ticket);

	fn validate(
		dest: &mut Option<MultiLocation>,
		xcm: &mut Option<Xcm<()>>,
	) -> SendResult<Self::Ticket> {
		// we won't have an access to `dest` and `xcm` in the `delvier` method, so precompute
		// everything required here
		let message_size = xcm
			.as_ref()
			.map(|xcm| xcm.encoded_size() as _)
			.ok_or(SendError::MissingArgument)?;

		// bridge doesn't support oversized/overweight messages now. So it is better to drop such
		// messages here than at the bridge hub. Let's check the message size.
		if message_size > HARD_MESSAGE_SIZE_LIMIT {
			return Err(SendError::ExceedsMaxMessageSize)
		}

		// just use exporter to validate destination and insert instructions to pay message fee
		// at the sibling/child bridge hub
		//
		// the cost will include both cost of: (1) to-sibling bridge hub delivery (returned by
		// the `Config::ToBridgeHubSender`) and (2) to-bridged bridge hub delivery (returned by
		// `Self::exporter_for`)
		ViaBridgeHubExporter::<T, I>::validate(dest, xcm)
			.map(|(ticket, cost)| ((message_size, ticket), cost))
	}

	fn deliver(ticket: Self::Ticket) -> Result<XcmHash, SendError> {
		// use router to enqueue message to the sibling/child bridge hub. This also should handle
		// payment for passing through this queue.
		let (message_size, ticket) = ticket;
		let xcm_hash = ViaBridgeHubExporter::<T, I>::deliver(ticket)?;

		// increase delivery fee factor if required
		Self::on_message_sent_to_bridge(message_size);

		Ok(xcm_hash)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use mock::*;

	use frame_support::traits::Hooks;
	use sp_runtime::traits::One;

	#[test]
	fn initial_fee_factor_is_one() {
		run_test(|| {
			assert_eq!(DeliveryFeeFactor::<TestRuntime, ()>::get(), MINIMAL_DELIVERY_FEE_FACTOR);
		})
	}

	#[test]
	fn fee_factor_is_not_decreased_from_on_initialize_when_queue_is_congested() {
		run_test(|| {
			DeliveryFeeFactor::<TestRuntime, ()>::put(FixedU128::from_rational(125, 100));
			TestLocalXcmChannelManager::make_congested();

			// it should not decrease, because queue is congested
			let old_delivery_fee_factor = XcmBridgeHubRouter::delivery_fee_factor();
			XcmBridgeHubRouter::on_initialize(One::one());
			assert_eq!(XcmBridgeHubRouter::delivery_fee_factor(), old_delivery_fee_factor);
		})
	}

	#[test]
	fn fee_factor_is_decreased_from_on_initialize_when_queue_is_uncongested() {
		run_test(|| {
			DeliveryFeeFactor::<TestRuntime, ()>::put(FixedU128::from_rational(125, 100));

			// it shold eventually decreased to one
			while XcmBridgeHubRouter::delivery_fee_factor() > MINIMAL_DELIVERY_FEE_FACTOR {
				XcmBridgeHubRouter::on_initialize(One::one());
			}

			// verify that it doesn't decreases anymore
			XcmBridgeHubRouter::on_initialize(One::one());
			assert_eq!(XcmBridgeHubRouter::delivery_fee_factor(), MINIMAL_DELIVERY_FEE_FACTOR);
		})
	}

	#[test]
	fn not_applicable_if_destination_is_within_other_network() {
		run_test(|| {
			assert_eq!(
				send_xcm::<XcmBridgeHubRouter>(
					MultiLocation::new(2, X2(GlobalConsensus(Rococo), Parachain(1000))),
					vec![].into(),
				),
				Err(SendError::NotApplicable),
			);
		});
	}

	#[test]
	fn exceeds_max_message_size_if_size_is_above_hard_limit() {
		run_test(|| {
			assert_eq!(
				send_xcm::<XcmBridgeHubRouter>(
					MultiLocation::new(2, X2(GlobalConsensus(Rococo), Parachain(1000))),
					vec![ClearOrigin; HARD_MESSAGE_SIZE_LIMIT as usize].into(),
				),
				Err(SendError::ExceedsMaxMessageSize),
			);
		});
	}

	#[test]
	fn returns_proper_delivery_price() {
		run_test(|| {
			let dest = MultiLocation::new(2, X1(GlobalConsensus(BridgedNetworkId::get())));
			let xcm: Xcm<()> = vec![ClearOrigin].into();
			let msg_size = xcm.encoded_size();

			// initially the base fee is used: `BASE_FEE + BYTE_FEE * msg_size + HRMP_FEE`
			let expected_fee = BASE_FEE + BYTE_FEE * (msg_size as u128) + HRMP_FEE;
			assert_eq!(
				XcmBridgeHubRouter::validate(&mut Some(dest), &mut Some(xcm.clone()))
					.unwrap()
					.1
					.get(0),
				Some(&(BridgeFeeAsset::get(), expected_fee).into()),
			);

			// but when factor is larger than one, it increases the fee, so it becomes:
			// `(BASE_FEE + BYTE_FEE * msg_size) * F + HRMP_FEE`
			let factor = FixedU128::from_rational(125, 100);
			DeliveryFeeFactor::<TestRuntime, ()>::put(factor);
			let expected_fee =
				(FixedU128::saturating_from_integer(BASE_FEE + BYTE_FEE * (msg_size as u128)) *
					factor)
					.into_inner() / FixedU128::DIV +
					HRMP_FEE;
			assert_eq!(
				XcmBridgeHubRouter::validate(&mut Some(dest), &mut Some(xcm)).unwrap().1.get(0),
				Some(&(BridgeFeeAsset::get(), expected_fee).into()),
			);
		});
	}

	#[test]
	fn sent_message_doesnt_increase_factor_if_queue_is_uncongested() {
		run_test(|| {
			let old_delivery_fee_factor = XcmBridgeHubRouter::delivery_fee_factor();
			assert_eq!(
				send_xcm::<XcmBridgeHubRouter>(
					MultiLocation::new(
						2,
						X2(GlobalConsensus(BridgedNetworkId::get()), Parachain(1000))
					),
					vec![ClearOrigin].into(),
				)
				.map(drop),
				Ok(()),
			);

			assert!(TestToBridgeHubSender::is_message_sent());
			assert_eq!(old_delivery_fee_factor, XcmBridgeHubRouter::delivery_fee_factor());
		});
	}

	#[test]
	fn sent_message_increases_factor_if_queue_is_congested() {
		run_test(|| {
			TestLocalXcmChannelManager::make_congested();

			let old_delivery_fee_factor = XcmBridgeHubRouter::delivery_fee_factor();
			assert_eq!(
				send_xcm::<XcmBridgeHubRouter>(
					MultiLocation::new(
						2,
						X2(GlobalConsensus(BridgedNetworkId::get()), Parachain(1000))
					),
					vec![ClearOrigin].into(),
				)
				.map(drop),
				Ok(()),
			);

			assert!(TestToBridgeHubSender::is_message_sent());
			assert!(old_delivery_fee_factor < XcmBridgeHubRouter::delivery_fee_factor());
		});
	}
}
