-- Annorex for Rex: review the replies and the files of a coding agent in a pane next to it.
--
-- `annorex rex install` writes this file beside Rex's init.lua. Then add to init.lua:
--
--   require("annorex").setup{}
--   rex.bind("cmd+shift+a", "annorex_last")
--   rex.bind("cmd+shift+f", "annorex_files")
--
-- and run `rex config reload`.
--
-- Options of setup{}:
--   bin        The annorex binary. Default "annorex" (found on the PATH of the Rex server).
--   placement  "auto", "right", or "below". Default: unset. Then config.toml decides, else "auto".
--
-- This module only starts `annorex rex open last` and `annorex rex open files`. The binary does all
-- other work.

local M = {}

M.options = {
  bin = "annorex",
}

-- The options that setup{} accepts.
local KNOWN = { bin = true, placement = true }

local function first_line(text)
  for line in (text or ""):gmatch("[^\n]+") do
    local trimmed = line:gsub("^%s+", ""):gsub("%s+$", "")
    if trimmed ~= "" then return trimmed end
  end
  return nil
end

local function fail(message)
  error("Annorex: " .. tostring(message), 0)
end

local function open_view(ctx, view)
  if ctx.block_id == nil or ctx.session_id == nil then fail("no focused pane") end
  local argv = {
    M.options.bin, "rex", "open", view,
    "--target", ctx.block_id,
    "--session", ctx.session_id,
  }
  if M.options.placement then
    table.insert(argv, "--placement")
    table.insert(argv, M.options.placement)
  end
  local result, err = rex.exec(argv, { timeout = 25 })
  if result == nil then fail(err) end
  if not result.ok then
    local line = first_line(result.stderr) or first_line(result.stdout) or "annorex rex open failed"
    -- The binary starts its errors with "annorex: ". fail() adds "Annorex: " again.
    fail((line:gsub("^annorex: ", "")))
  end
  return nil
end

local function open_last(ctx) return open_view(ctx, "last") end
local function open_files(ctx) return open_view(ctx, "files") end

function M.setup(opts)
  for key, value in pairs(opts or {}) do
    if KNOWN[key] then
      M.options[key] = value
    else
      rex.log("warn", "annorex: unknown option " .. tostring(key))
    end
  end

  rex.action{
    name = "annorex_last",
    title = "Annorex: review the last reply",
    description = "Open the last reply of the agent in the focused pane, next to it.",
    category = "Annorex",
    keywords = "annorex review reply agent claude pi opencode",
    run = function(ctx) return open_last(ctx) end,
  }
  rex.action{
    name = "annorex_files",
    title = "Annorex: review files",
    description = "Open the files of the work directory of the agent in the focused pane, next to it.",
    category = "Annorex",
    keywords = "annorex review files tree agent claude pi opencode",
    run = function(ctx) return open_files(ctx) end,
  }
  return M
end

return M
