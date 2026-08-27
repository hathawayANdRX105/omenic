# omenic

<!-- demo note (issue #75): compliance gate deployed -->
Omenic is a task-driven agent orchestrator where agents act as functions following Prompt → Result.

## Commands

`cargo install --path .` installs both `omenic` and the short alias `cli`.

```bash
oi plan
oi task add "implement login"
```

`cli` and `omenic` invoke the same binary entry point and accept the same arguments. Remove the alias by uninstalling the package or deleting the installed `cli` binary from Cargo's bin directory.
