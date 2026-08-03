//! Claude compact workaround (no native endpoint — summarize via execute).

use crate::error::AppResult;
use crate::providers::compact::{ExecCompactRequest, ExecCompactResponse};
use crate::providers::execute::ExecStream;

use super::ClaudeProvider;

impl ClaudeProvider {
  /// No native compact endpoint — summarize via [`Self::execute`].
  pub async fn execute_compact(&self, req: &ExecCompactRequest) -> AppResult<ExecCompactResponse> {
    let mut summary = req.as_summary_exec_request();
    summary.stream = false;
    let result = self.execute(&summary).await?;
    Ok(ExecCompactResponse::from_exec(result))
  }

  /// No native compact stream — summarize via [`Self::execute_stream`].
  pub async fn execute_compact_stream(&self, req: &ExecCompactRequest) -> AppResult<ExecStream> {
    let mut summary = req.as_summary_exec_request();
    summary.stream = true;
    self.execute_stream(&summary).await
  }
}
