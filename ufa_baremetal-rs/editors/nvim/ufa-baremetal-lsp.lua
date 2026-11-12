-- Neovim integration for ufa-baremetal-lsp.
--
-- Uses Neovim's built-in LSP client (`vim.lsp.start`) directly -- no
-- nvim-lspconfig or any other plugin required, matching this project's
-- own "stdlib only, no dependency you don't need" discipline. Works on
-- any Neovim with an LSP client (0.5+; the `vim.lsp.config`/`vim.lsp.enable`
-- path below is 0.11+ and preferred when available).
--
-- INSTALL
--   1. `cargo install --path bin/ufa-baremetal-lsp` from the workspace
--      root (or just `cargo build --release -p ufa-baremetal-lsp` and
--      point `cmd` below at the resulting
--      `target/release/ufa-baremetal-lsp` binary directly).
--   2. Copy this file to e.g. `~/.config/nvim/lua/ufa_baremetal_lsp.lua`
--      and `require("ufa_baremetal_lsp").setup()` from your `init.lua`
--      -- OR paste the relevant block below straight into your config.
--
-- Assembly files don't get a filetype Neovim recognizes out of the box
-- as "this project's dialect" -- adjust the `pattern`/`ft_pattern` below
-- if your `.s` files should only trigger this for a specific directory
-- (e.g. only under a `codegen/` output tree) rather than every `*.s` you
-- ever open.

local M = {}

function M.setup(opts)
  opts = opts or {}
  local cmd = opts.cmd or { "ufa-baremetal-lsp" } -- must resolve on $PATH, or give an absolute path

  vim.filetype.add({ extension = { s = "asm" } })

  local augroup = vim.api.nvim_create_augroup("UfaBaremetalLsp", { clear = true })
  vim.api.nvim_create_autocmd("FileType", {
    group = augroup,
    pattern = opts.filetype or "asm",
    callback = function(args)
      vim.lsp.start({
        name = "ufa-baremetal-lsp",
        cmd = cmd,
        root_dir = vim.fs.root(args.buf, { ".git", "Cargo.toml" }) or vim.fn.getcwd(),
      })
    end,
  })
end

return M

-- ---------------------------------------------------------------------
-- Neovim 0.11+ alternative: the newer declarative `vim.lsp.config` /
-- `vim.lsp.enable` API (uncomment and use instead of `setup()` above if
-- you're on 0.11+ and prefer this style):
--
-- vim.lsp.config["ufa_baremetal_lsp"] = {
--   cmd = { "ufa-baremetal-lsp" },
--   filetypes = { "asm" },
--   root_markers = { ".git", "Cargo.toml" },
-- }
-- vim.lsp.enable("ufa_baremetal_lsp")
