-- Annorex for Rex: review the replies and the files of a coding agent over the full window of the agent.
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
--   bin           The annorex binary: a path, or a name that Rex looks up on its own PATH.
--                 Default: found as described below.
--   update_check  false: the viewer does not check GitHub for a newer release. Default: true.
--
-- Rex runs actions with a short PATH (/usr/bin:/bin:/usr/sbin:/sbin), not the PATH of your shell.
-- So without bin, the module uses the first of these files that exists:
--   1. the binary that ran `annorex rex install`
--   2. $CARGO_HOME/bin/annorex, else ~/.cargo/bin/annorex (cargo install)
--   3. /opt/homebrew/bin/annorex and /usr/local/bin/annorex (Homebrew)
--   4. ~/.local/bin/annorex
--
-- This module only starts `annorex rex open last` and `annorex rex open files`. The binary does all
-- other work.

local M = {}

M.options = {
  bin = nil,
  update_check = true,
}

local INSTALLED_BIN = nil -- set by annorex rex install

-- The options that setup{} accepts.
local KNOWN = { bin = true, update_check = true }

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

-- True when path is a file. io.open also opens a directory, so "path/." is tried too: it opens
-- only when path is a directory.
local function exists(path)
  local file = io.open(path, "r")
  if file == nil then return false end
  file:close()
  local dir = io.open(path .. "/.", "r")
  if dir ~= nil then
    dir:close()
    return false
  end
  return true
end

-- The places to look for the binary when setup{} gives no bin, in order.
local function candidates()
  local list = {}
  if INSTALLED_BIN ~= nil and INSTALLED_BIN ~= "" then list[#list + 1] = INSTALLED_BIN end
  local home = os.getenv("HOME")
  local cargo_home = os.getenv("CARGO_HOME")
  if cargo_home ~= nil and cargo_home ~= "" then
    list[#list + 1] = cargo_home .. "/bin/annorex"
  elseif home ~= nil and home ~= "" then
    list[#list + 1] = home .. "/.cargo/bin/annorex"
  end
  list[#list + 1] = "/opt/homebrew/bin/annorex"
  list[#list + 1] = "/usr/local/bin/annorex"
  if home ~= nil and home ~= "" then list[#list + 1] = home .. "/.local/bin/annorex" end
  return list
end

-- The binary to start. A bin from setup{} wins: a path must exist, a bare name goes to the PATH.
local function binary()
  local bin = M.options.bin
  if bin ~= nil and bin ~= "" then
    if bin:find("/", 1, true) and not exists(bin) then
      fail("the annorex binary " .. bin .. " from setup{} does not exist")
    end
    return bin
  end
  local tried = candidates()
  for _, path in ipairs(tried) do
    if exists(path) then return path end
  end
  fail("annorex was not found; tried: " .. table.concat(tried, ", ")
    .. '. Give the path with setup{ bin = "..." }.')
end

local function open_view(ctx, view)
  if ctx.block_id == nil or ctx.session_id == nil then fail("no focused pane") end
  local argv = {
    binary(), "rex", "open", view,
    "--target", ctx.block_id,
    "--session", ctx.session_id,
  }
  if M.options.update_check == false then argv[#argv + 1] = "--no-update-check" end
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
    description = "Open the last reply of the agent in the focused pane, over the full window.",
    category = "Annorex",
    keywords = "annorex review reply agent claude pi opencode",
    run = function(ctx) return open_last(ctx) end,
  }
  rex.action{
    name = "annorex_files",
    title = "Annorex: review files",
    description = "Open the files of the work directory of the agent in the focused pane, over the full window.",
    category = "Annorex",
    keywords = "annorex review files tree agent claude pi opencode",
    run = function(ctx) return open_files(ctx) end,
  }
  return M
end

return M
