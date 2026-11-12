# Cargo: Override inherited default-features

## Summary

- Starting with Rust 1.99, `default-features` on an inherited dependency in a 2024 Edition package overrides the value from `[workspace.dependencies]`.

## Details

[Workspace inheritance] allows you to specify dependencies in one place (the workspace), and then to refer to those workspace dependencies from within a package.

In editions earlier than 2024, `default-features = false` is ignored when the workspace dependency specifies `default-features = true` (or does not specify `default-features`). The original 2024 Edition behavior in Rust 1.85 through 1.98 rejected this combination. Starting with Rust 1.99, a 2024 Edition package's `default-features` setting overrides the workspace setting.

For example, with a workspace that specifies:

```toml
[workspace.dependencies]
regex = "1.10.4"
```

The following disables the default features for `regex`:

```toml
[package]
name = "foo"
version = "1.0.0"
edition = "2024"

[dependencies]
regex = { workspace = true, default-features = false }
```

Just beware that if you build multiple workspace members at the same time, the features will be unified so that if one member sets `default-features = true` (which is the default if not explicitly set), the default-features will be enabled for all members using that dependency.

## Migration

When using `cargo fix --edition`, Cargo will automatically remove `default-features = false` in this situation. Without this fix, changing to the 2024 Edition would produce an error with Rust 1.85 through 1.98 and would disable default features with Rust 1.99 or newer.

If you want to use the new behavior, add the setting back after changing to the 2024 Edition. If you prefer to migrate manually while preserving the previous behavior, remove the corresponding entries. Previous editions should display something like:

```text
warning: /home/project/Cargo.toml: `default-features` is ignored for regex,
since `default-features` was not specified for `workspace.dependencies.regex`;
overriding workspace `default-features` to false requires Rust 1.99+
and the 2024 edition
```

[workspace inheritance]: ../../cargo/reference/specifying-dependencies.html#inheriting-a-dependency-from-a-workspace
