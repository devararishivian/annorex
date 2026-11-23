-- Annorex for Rex: review the replies and the files of a coding agent over the full window of the agent.
--
-- `annorex rex install` writes this file beside Rex's init.lua, adds these lines to init.lua,
-- and runs `rex config reload`:
--
--   require("annorex").setup{}
--   rex.bind("cmd+shift+a", "annorex_last")
--   rex.bind("cmd+shift+f", "annorex_files")
--
-- `annorex rex install --no-init` writes only this file. Then add the lines yourself.
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
-- This module only starts `annorex rex open last`, `annorex rex open files`, and `annorex rex agents`.
-- When the focused pane runs no supported agent, it asks in the command palette which agent pane to open.
-- The binary does all other work.

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

-- The exit code of `annorex rex open` when the target pane runs no supported agent.
local NOT_AGENT_CODE = 5

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

-- The first line of the output of a failed command. The binary starts its errors with "annorex: ".
-- fail() adds "Annorex: " again.
local function error_line(result, default)
  local line = first_line(result.stderr) or first_line(result.stdout) or default
  return (line:gsub("^annorex: ", ""))
end

-- Run `annorex rex open VIEW` for the agent pane target.
local function run_open(ctx, view, target)
  local argv = {
    binary(), "rex", "open", view,
    "--target", target,
    "--session", ctx.session_id,
  }
  if M.options.update_check == false then argv[#argv + 1] = "--no-update-check" end
  local result, err = rex.exec(argv, { timeout = 25 })
  if result == nil then fail(err) end
  return result
end

local function check_open(result)
  if not result.ok then fail(error_line(result, "annorex rex open failed")) end
  return nil
end

-- The agent panes of the session, from `annorex rex agents`.
local function agent_panes(ctx)
  local result, err = rex.exec({ binary(), "rex", "agents", "--session", ctx.session_id }, { timeout = 25 })
  if result == nil then fail(err) end
  if not result.ok then fail(error_line(result, "annorex rex agents failed")) end
  local list, why = rex.json.decode(result.stdout)
  if type(list) ~= "table" or type(list.agents) ~= "table" then
    fail("cannot read the list of agent panes: " .. tostring(why))
  end
  return list.agents
end

-- Ask in the command palette which agent pane to open. The title gives the reason for the question.
-- Returns its block id, or nil and the reason.
-- rex.ui.pick works only in the run of an action and in the functions that it calls directly, not in
-- pcall.
local function pick_agent(agents, title)
  local items = {}
  for index, agent in ipairs(agents) do
    items[index] = {
      id = agent.block_id,
      title = agent.title,
      subtitle = agent.subtitle,
      detail = agent.detail,
      icon = "terminal",
    }
  end
  return rex.ui.pick{
    title = title,
    placeholder = "Filter agents",
    confirm = "Open",
    items = items,
  }
end

local function open_view(ctx, view)
  if ctx.block_id == nil or ctx.session_id == nil then fail("no focused pane") end
  local result = run_open(ctx, view, ctx.block_id)
  if result.code ~= NOT_AGENT_CODE then return check_open(result) end
  -- The focused pane runs no supported agent. Say so and ask which agent pane of the session to open,
  -- also when there is only one. Never go to another pane without an answer.
  local not_agent = error_line(result, "this pane runs no supported agent")
  local agents = agent_panes(ctx)
  if #agents == 0 then
    fail(not_agent .. ". No pane of this session runs Claude Code, Pi, OpenCode, or Codex.")
  end
  local id, why = pick_agent(agents, "Annorex: " .. not_agent .. ". Pick an agent")
  if id == nil then
    if why == "cancelled" then return nil end
    fail(not_agent .. " (no answer from the command palette: " .. tostring(why) .. ")")
  end
  -- The viewer opens in the window of the agent pane. A failed focus does not stop the open.
  rex.session.focus_block{ block_id = id }
  return check_open(run_open(ctx, view, id))
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
    keywords = "annorex review reply agent claude pi opencode codex",
    run = function(ctx) return open_last(ctx) end,
  }
  rex.action{
    name = "annorex_files",
    title = "Annorex: review files",
    description = "Open the files of the work directory of the agent in the focused pane, over the full window.",
    category = "Annorex",
    keywords = "annorex review files tree agent claude pi opencode codex",
    run = function(ctx) return open_files(ctx) end,
  }
  return M
end

return M
