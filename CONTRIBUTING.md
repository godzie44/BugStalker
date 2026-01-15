# Contributing to BugStalker

You likely arrived here from the Issues or Pull Requests page. 
Below are some suggestions to help make your contribution as effective as possible.

## Filing an issue

Consider using a bug report or feature request template. You're welcome to adapt it to your needs. 
While you can opt for a blank issue instead, please ensure you provide complete and clear information.

Feature requests generally fall into two categories:

* **Enhancements to existing functionality** or other straightforward, ready-to-implement requests.
* **New ideas or proposals** requiring discussion.

The former is completely okay to be asked via an issue.

## Opening a pull request

`BugStalker` development model prioritizes two key objectives:

* Rapid updates to support new rustc versions as soon as possible.
* Feature development that doesn't interfere with compiler version support.

To achieve this, we maintain two release types:

* **Minor releases** (frequent):
Include fixes, small features, and new compiler version support.

* **Major releases** (less frequent):
Introduce significant features with longer development cycles.

We follow semantic versioning (X.Y.Z):
* X or Y increments indicate major releases
* Z increments indicate minor releases

### Feature Development Guidelines

For major features:
* Hide behind the `nightly` feature flag until the major release
* Use the `ui::console::cfg::nightly` macro to conceal console UI elements
* Ensure your implementation follows this pattern if contributing substantial changes

## Adding support for new compiler version, checklist

- [ ] add a new rustc version into `src/version.rs`
- [ ] update `.github/workflows/ci.yml` by adding a new version for integration and functional tests
- [ ] make sure the tests are not broken (in github ci too), correct test or sources if necessary
- [ ] fix `cargo fmt` and `cargo clippy` if needed
- [ ] add information about new version into website and `CHANGELOG.md`
- [ ] create and submit a pool request
- [ ] create a new release from master branch

## Testing DAP extension in VSCode

For easiest test, add this code in `.vscode/launch.json`:

```
{
  "version": "0.2.0",
  "configurations": [
    {
      "name": "Run Extension",
      "type": "extensionHost",
      "request": "launch",
      "args": ["--extensionDevelopmentPath=${workspaceFolder}/extension/vscode"],
      "outFiles": ["${workspaceFolder}/extension/vscode/out/**/*.js"],
      "preLaunchTask": "${defaultBuildTask}",
      "env": {
        "BUGSTALKER_DIR": "${workspaceFolder}"
      }
    }
  ]
}
```

and this into `.vscode/tasks.json`:

```
{
  "version": "2.0.0",
  "tasks": [
    {
      "type": "npm",
      "script": "watch",
      "problemMatcher": "$tsc-watch",
      "isBackground": true,
      "presentation": {
        "reveal": "never"
      },
      "group": {
        "kind": "build",
        "isDefault": true
      },
      "options": {
        "cwd": "${workspaceFolder}/extension"
      }
    }
  ]
}
```

Now you can test the DAP extension by pressing F5 in VSCode.

## Note on Deprecated Development Model

The previous GitFlow-like model (see doc/flow.png) has been discontinued. Refer to #64 for details on this decision.
