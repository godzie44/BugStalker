"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.activate = activate;
exports.deactivate = deactivate;
// The module 'vscode' contains the VS Code extensibility API
// Import the module and reference it with the alias vscode in your code below
var vscode = require("vscode");
// This method is called when your extension is activated
// Your extension is activated the very first time the command is executed
function activate(context) {
    // Use the console to output diagnostic information (console.log) and errors (console.error)
    // This line of code will only be executed once when your extension is activated
    console.log('Congratulations, your extension "bugstalker" is now active!');
    // The command has been defined in the package.json file
    // Now provide the implementation of the command with registerCommand
    // The commandId parameter must match the command field in package.json
    var disposable = vscode.commands.registerCommand("bugstalker.helloWorld", function () {
        // The code you place here will be executed every time your command is executed
        // Display a message box to the user
        vscode.window.showInformationMessage("Hello World from BugStalker!");
    });
    context.subscriptions.push(disposable);
    context.subscriptions.push(vscode.debug.registerDebugAdapterDescriptorFactory("bugstalker", new DebugAdapterExecutableFactory()));
}
var DebugAdapterExecutableFactory = /** @class */ (function () {
    function DebugAdapterExecutableFactory() {
    }
    DebugAdapterExecutableFactory.prototype.createDebugAdapterDescriptor = function (session, executable) {
        console.log("Starting debug adapter", executable);
        // TODO: This should run a pre-built binary instead of cargo
        return new vscode.DebugAdapterExecutable("cargo", ["run", "-q", "--", "--dap"], {
            cwd: process.env.BUGSTALKER_DIR,
            env: {
                RUST_LOG: "info,bugstalker=debug",
            },
        });
    };
    return DebugAdapterExecutableFactory;
}());
// This method is called when your extension is deactivated
function deactivate() { }
