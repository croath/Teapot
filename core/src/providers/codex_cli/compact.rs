//! Codex CLI compact — summarize via execute (app-server has no HTTP compact).

use crate::error::AppResult;
use crate::providers::compact::{ExecCompactRequest, ExecCompactResponse};
use crate::providers::execute::ExecStream;

use super::CodexCliProvider;

impl CodexCliProvider {
  pub async fn execute_compact(&self, req: &ExecCompactRequest) -> AppResult<ExecCompactResponse> {
    let mut summary = req.as_summary_exec_request();
    summary.stream = false;
    let result = self.execute(&summary).await?;
    Ok(ExecCompactResponse::from_exec(result))
  }

  pub async fn execute_compact_stream(&self, req: &ExecCompactRequest) -> AppResult<ExecStream> {
    let mut summary = req.as_summary_exec_request();
    summary.stream = true;
    self.execute_stream(&summary).await
  }
}
