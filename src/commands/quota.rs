use anstream::println;
use anyhow::Result;
use clap::Args;
use serde_json::Value;

use crate::lark::LarkCli;

#[derive(Debug, Args)]
pub struct QuotaCommand {
    /// Current user's user_id for drive quota_details.get.
    #[arg(long)]
    user_id: String,
}

impl QuotaCommand {
    pub async fn run(self) -> Result<()> {
        let lark = LarkCli::new();
        let params = serde_json::json!({ "quota_detail_id": self.user_id }).to_string();
        let obj = lark
            .json_with_retry([
                "drive",
                "quota_details",
                "get",
                "--as",
                "user",
                "--params",
                params.as_str(),
                "--format",
                "json",
            ])
            .await?;
        let json = serde_json::to_string_pretty(&extract_data(&obj))?;
        println!("{}", crate::highlight::json(&json).unwrap_or(json));
        Ok(())
    }
}

fn extract_data(value: &Value) -> Value {
    value.get("data").cloned().unwrap_or_else(|| value.clone())
}
