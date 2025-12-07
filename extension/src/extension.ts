// The module 'vscode' contains the VS Code extensibility API
// Import the module and reference it with the alias vscode in your code below
import * as vscode from "vscode";

// This method is called when your extension is activated
// Your extension is activated the very first time the command is executed
export function activate(context: vscode.ExtensionContext) {
	// Use the console to output diagnostic information (console.log) and errors (console.error)
	// This line of code will only be executed once when your extension is activated
	console.log('Congratulations, your extension "bugstalker" is now active!');

	// The command has been defined in the package.json file
	// Now provide the implementation of the command with registerCommand
	// The commandId parameter must match the command field in package.json
	const disposable = vscode.commands.registerCommand(
		"bugstalker.helloWorld",
		() => {
			// The code you place here will be executed every time your command is executed
			// Display a message box to the user
			vscode.window.showInformationMessage("Hello World from BugStalker!");
		},
	);

	context.subscriptions.push(disposable);

	context.subscriptions.push(
		vscode.debug.registerDebugAdapterDescriptorFactory(
			"bugstalker",
			new DebugAdapterExecutableFactory(),
		),
	);
}

class DebugAdapterExecutableFactory
	implements vscode.DebugAdapterDescriptorFactory
{
	createDebugAdapterDescriptor(
		session: vscode.DebugSession,
		executable: vscode.DebugAdapterExecutable | undefined,
	): vscode.ProviderResult<vscode.DebugAdapterDescriptor> {
		console.log("Starting debug adapter", executable);

		const config = session.configuration;
		let inDebug = config?.debugMode;

		let bin = inDebug ? "cargo" : "bs";
        let args = inDebug ? ["run", "-q", "--", "--dap"] : ["--dap"];
		let env = config?.env;
		env["RUST_LOG"] = inDebug ? "info,bugstalker=debug" : "info,bugstalker=info";

		return new vscode.DebugAdapterExecutable(
			bin,
			args,
			{
				cwd: config.cwd ?? session.workspaceFolder?.uri.fsPath,
				env,
			},
		);
	}
}

// This method is called when your extension is deactivated
export function deactivate() {}
