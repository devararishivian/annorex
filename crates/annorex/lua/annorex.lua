-- Annorex for Rex: review the replies of a coding agent in a pane next to it.
--
-- `annorex rex install` writes this file beside Rex's init.lua. Then add to init.lua:
--
--   require("annorex").setup{}
--   rex.bind("cmd+shift+a", "annorex_last")
--
-- and run `rex config reload`.
--
-- Options of setup{}:
--   bin        The annorex binary. Default "annorex" (found on the PATH of the Rex server).
--   placement  "auto" (default), "right", or "below".
--
-- This module only starts `annorex rex open last`. The binary does all other work.

local M = {}

M.options = {
  bin = "annorex",
  placement = "auto",
}

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

local function open_last(ctx)
  if ctx.block_id == nil or ctx.session_id == nil then fail("no focused pane") end
  local result, err = rex.exec({
    M.options.bin, "rex", "open", "last",
    "--target", ctx.block_id,
    "--session", ctx.session_id,
    "--placement", M.options.placement,
  }, { timeout = 25 })
  if result == nil then fail(err) end
  if not result.ok then
    fail(first_line(result.stderr) or first_line(result.stdout) or "annorex rex open failed")
  end
  return nil
end

function M.setup(opts)
  for key, value in pairs(opts or {}) do
    if M.options[key] ~= nil then
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
  return M
end

return M
