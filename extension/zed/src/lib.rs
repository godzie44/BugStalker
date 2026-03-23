use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use zed_extension_api::{
    self as zed, DebugAdapterBinary, StartDebuggingRequestArguments,
    StartDebuggingRequestArgumentsRequest, serde_json,
};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BsDebugConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    program: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    request: String,
}

struct BsExtension;

impl BsExtension {
    pub const NAME: &'static str = "bs";
}

impl zed::Extension for BsExtension {
    fn new() -> Self
    where
        Self: Sized,
    {
        BsExtension {}
    }

    fn get_dap_binary(
        &mut self,
        adapter_name: String,
        config: zed_extension_api::DebugTaskDefinition,
        user_provided_debug_adapter_path: Option<String>,
        worktree: &zed_extension_api::Worktree,
    ) -> Result<zed_extension_api::DebugAdapterBinary, String> {
        if config.adapter != Self::NAME {
            return Err(format!(
                "BS extension does not support unknown adapter in `get_dap_binary`: {} (supported: [{}])",
                adapter_name,
                Self::NAME
            ));
        }

        let configuration = config.config.to_string();

        let config: BsDebugConfig =
            serde_json::from_str(&config.config).map_err(|e| e.to_string())?;

        let request = match config.request.as_str() {
            "launch" => StartDebuggingRequestArgumentsRequest::Launch,
            "attach" => StartDebuggingRequestArgumentsRequest::Attach,
            other => {
                return Err(format!(
                    "Unexpected value for `request` key in debug adapter configuration: {other:?}"
                ));
            }
        };

        let (command, arguments) = user_provided_debug_adapter_path
            // TODO how to get arguments if user_provided_debug_adapter_path is defined? (currently its hardcoded)
            .map(|path| (path, vec!["--dap".into()]))
            .or_else(|| {
                let bs = worktree.which("bs")?;
                Some((
                    bs,
                    vec![
                        "--dap".into(),
                        "--dap-log-file".into(),
                        "/tmp/bs-zed-1.log".into(),
                    ],
                ))
            })
            .ok_or_else(|| "Could not find bs".to_owned())?;

        Ok(DebugAdapterBinary {
            command: Some(command),
            arguments: arguments,
            connection: None,
            envs: config.env.into_iter().collect(),
            cwd: Some(config.cwd.unwrap_or_else(|| worktree.root_path())),
            request_args: StartDebuggingRequestArguments {
                request: request,
                configuration: configuration.to_string(),
            },
        })
    }

    fn dap_request_kind(
        &mut self,
        adapter_name: String,
        config: serde_json::Value,
    ) -> Result<StartDebuggingRequestArgumentsRequest, String> {
        if adapter_name != Self::NAME {
            return Err(format!(
                "BS extension does not support unknown adapter in `dap_request_kind`: {adapter_name} (supported: [{}])",
                Self::NAME
            ));
        }

        let config: BsDebugConfig = serde_json::from_value(config).map_err(|e| e.to_string())?;

        let request = match config.request.as_str() {
            "launch" => StartDebuggingRequestArgumentsRequest::Launch,
            "attach" => StartDebuggingRequestArgumentsRequest::Attach,
            other => {
                return Err(format!(
                    "Unexpected value for `request` key in debug adapter configuration: {other:?}"
                ));
            }
        };

        Ok(request)
    }

    fn dap_config_to_scenario(
        &mut self,
        zed_scenario: zed_extension_api::DebugConfig,
    ) -> Result<zed_extension_api::DebugScenario, String> {
        if zed_scenario.adapter != Self::NAME {
            return Err(format!(
                "BS extension does not support unknown adapter in `dap_config_to_scenario`: {} (supported: [{}])",
                zed_scenario.adapter,
                Self::NAME
            ));
        }

        match zed_scenario.request {
            zed_extension_api::DebugRequest::Launch(launch) => {
                let config = serde_json::to_string(&BsDebugConfig {
                    program: Some(launch.program),
                    env: launch.envs.into_iter().collect(),
                    cwd: launch.cwd.clone(),
                    request: "launch".to_owned(),
                    pid: None,
                })
                .unwrap();

                Ok(zed_extension_api::DebugScenario {
                    adapter: zed_scenario.adapter,
                    label: zed_scenario.label,
                    config,
                    tcp_connection: None,
                    build: None,
                })
            }
            zed_extension_api::DebugRequest::Attach(attach) => {
                let config = serde_json::to_string(&BsDebugConfig {
                    program: None,
                    env: Default::default(),
                    request: "attach".to_owned(),
                    pid: attach.process_id,
                    cwd: None,
                })
                .unwrap();

                Ok(zed_extension_api::DebugScenario {
                    adapter: zed_scenario.adapter,
                    label: zed_scenario.label,
                    build: None,
                    config,
                    tcp_connection: None,
                })
            }
        }
    }
}

zed::register_extension!(BsExtension);
