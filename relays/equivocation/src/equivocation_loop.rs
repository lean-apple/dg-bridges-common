// Copyright 2019-2023 Parity Technologies (UK) Ltd.
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

use crate::{
	handle_client_error, reporter::EquivocationsReporter, EquivocationDetectionPipeline,
	SourceClient, TargetClient,
};

use crate::block_checker::BlockChecker;
use finality_relay::{FinalityProofsBuf, FinalityProofsStream};
use futures::{select, FutureExt};
use num_traits::Saturating;
use relay_utils::{metrics::MetricsParams, FailedClient};
use std::{future::Future, time::Duration};

/// Equivocations detection loop state.
struct EquivocationDetectionLoop<
	P: EquivocationDetectionPipeline,
	SC: SourceClient<P>,
	TC: TargetClient<P>,
> {
	source_client: SC,
	target_client: TC,

	from_block_num: Option<P::TargetNumber>,
	until_block_num: Option<P::TargetNumber>,

	reporter: EquivocationsReporter<P, SC>,

	finality_proofs_stream: FinalityProofsStream<P, SC>,
	finality_proofs_buf: FinalityProofsBuf<P>,
}

impl<P: EquivocationDetectionPipeline, SC: SourceClient<P>, TC: TargetClient<P>>
	EquivocationDetectionLoop<P, SC, TC>
{
	async fn ensure_finality_proofs_stream(&mut self) {
		match self.finality_proofs_stream.ensure_stream(&self.source_client).await {
			Ok(_) => {},
			Err(e) => {
				log::error!(
					target: "bridge",
					"Could not connect to the {} `FinalityProofsStream`: {e:?}",
					P::SOURCE_NAME,
				);

				// Reconnect to the source client if needed
				handle_client_error(&mut self.source_client, e).await;
			},
		}
	}

	async fn best_finalized_target_block_number(&mut self) -> Option<P::TargetNumber> {
		match self.target_client.best_finalized_header_number().await {
			Ok(block_num) => Some(block_num),
			Err(e) => {
				log::error!(
					target: "bridge",
					"Could not read best finalized header number from {}: {e:?}",
					P::TARGET_NAME,
				);

				// Reconnect target client and move on
				handle_client_error(&mut self.target_client, e).await;

				None
			},
		}
	}

	async fn do_run(&mut self, tick: Duration, exit_signal: impl Future<Output = ()>) {
		let exit_signal = exit_signal.fuse();
		futures::pin_mut!(exit_signal);

		loop {
			// Make sure that we are connected to the source finality proofs stream.
			self.ensure_finality_proofs_stream().await;
			// Check the status of the pending equivocation reports
			self.reporter.process_pending_reports().await;

			// Update blocks range.
			if let Some(block_number) = self.best_finalized_target_block_number().await {
				self.from_block_num.get_or_insert(block_number);
				self.until_block_num = Some(block_number);
			}
			let (from, until) = match (self.from_block_num, self.until_block_num) {
				(Some(from), Some(until)) => (from, until),
				_ => continue,
			};

			// Check the available blocks
			let mut current_block_number = from;
			while current_block_number <= until {
				self.finality_proofs_buf.fill(&mut self.finality_proofs_stream);
				let block_checker = BlockChecker::new(current_block_number);
				let _ = block_checker
					.run(
						&mut self.source_client,
						&mut self.target_client,
						&mut self.finality_proofs_buf,
						&mut self.reporter,
					)
					.await;
				current_block_number = current_block_number.saturating_add(1.into());
			}
			self.until_block_num = Some(current_block_number);

			select! {
				_ = async_std::task::sleep(tick).fuse() => {},
				_ = exit_signal => return,
			}
		}
	}

	pub async fn run(
		source_client: SC,
		target_client: TC,
		tick: Duration,
		exit_signal: impl Future<Output = ()>,
	) -> Result<(), FailedClient> {
		let mut equivocation_detection_loop = Self {
			source_client,
			target_client,
			from_block_num: None,
			until_block_num: None,
			reporter: EquivocationsReporter::<P, SC>::new(),
			finality_proofs_stream: FinalityProofsStream::new(),
			finality_proofs_buf: FinalityProofsBuf::new(vec![]),
		};

		equivocation_detection_loop.do_run(tick, exit_signal).await;
		Ok(())
	}
}

/// Spawn the equivocations detection loop.
pub async fn run<P: EquivocationDetectionPipeline>(
	source_client: impl SourceClient<P>,
	target_client: impl TargetClient<P>,
	tick: Duration,
	metrics_params: MetricsParams,
	exit_signal: impl Future<Output = ()> + 'static + Send,
) -> Result<(), relay_utils::Error> {
	let exit_signal = exit_signal.shared();
	relay_utils::relay_loop(source_client, target_client)
		.with_metrics(metrics_params)
		.expose()
		.await?
		.run(
			format!("{}_to_{}_EquivocationDetection", P::SOURCE_NAME, P::TARGET_NAME),
			move |source_client, target_client, _metrics| {
				EquivocationDetectionLoop::run(
					source_client,
					target_client,
					tick,
					exit_signal.clone(),
				)
			},
		)
		.await
}
