-- `C` casts the current file to a DLNA TV picked from a menu. While casting,
-- the TV plays whatever file mpv has loaded, mpv stays paused and its
-- playback keys drive the TV, and when the TV finishes a file mpv moves on
-- to the next one in the playlist. When the cast ends, mpv is left at the
-- TV's position. `C` again ends the cast.

local input = require("mp.input")

local tvcast = "@tvcast@"
-- The TV being cast to, and an id that tells the current cast's exit apart
-- from that of one it replaced.
local tv, session = nil, 0

local remote = {
    SPACE = { "toggle" },
    p = { "toggle" },
    LEFT = { "seek", "-5" },
    RIGHT = { "seek", "+5" },
    UP = { "seek", "+60" },
    DOWN = { "seek", "-60" },
}

local function trim(s)
    return ((s or ""):gsub("%s+$", ""))
end

-- Runs tvcast with args; synchronously without a callback.
local function tvcast_run(args, callback)
    local argv = { tvcast }
    for _, arg in ipairs(args) do
        table.insert(argv, arg)
    end
    local cmd = {
        name = "subprocess",
        args = argv,
        capture_stdout = true,
        capture_stderr = true,
        playback_only = false,
    }
    if callback then
        mp.command_native_async(cmd, function(_, r)
            callback(r)
        end)
    else
        return mp.command_native(cmd)
    end
end

local function set_remote(enabled)
    for key, args in pairs(remote) do
        if enabled then
            mp.add_forced_key_binding(key, "tvcast-" .. key, function()
                tvcast_run(args, function(r)
                    mp.osd_message("TV: " .. trim(r.status == 0 and r.stdout or r.stderr))
                end)
            end)
        else
            mp.remove_key_binding("tvcast-" .. key)
        end
    end
end

local function has_next()
    return mp.get_property_number("playlist-pos", 0) + 1 < mp.get_property_number("playlist-count", 0)
end

local function ended(id, r)
    if id ~= session then
        return
    end
    local pos = tonumber(trim(r.stdout))
    local finished = pos and pos >= mp.get_property_number("duration", math.huge) - 5
    if tv and finished and has_next() then
        return mp.command("playlist-next")
    end
    tv = nil
    set_remote(false)
    if pos then
        mp.commandv("seek", pos, "absolute")
    end
    if r.status == 0 then
        mp.osd_message("Cast ended")
    else
        mp.osd_message("Cast failed: " .. trim(r.stderr), 5)
    end
end

-- Casts the current file to tv, replacing any cast already running.
local function start()
    local args = { "play", "-t", tv.url, "-start", tostring(math.floor(mp.get_property_number("time-pos", 0))) }
    local sub = mp.get_property_native("current-tracks/sub")
    local subfile = sub and sub.external and sub["external-filename"]
    local ext = subfile and subfile:lower():match("%.(%w+)$")
    if ext == "srt" or ext == "vtt" then
        table.insert(args, "-s")
        table.insert(args, subfile)
    end
    table.insert(args, "--")
    table.insert(args, mp.get_property("path"))

    tvcast_run({ "stop" })
    session = session + 1
    local id = session
    mp.set_property_bool("pause", true)
    set_remote(true)
    mp.osd_message("Casting to " .. tv.name)
    tvcast_run(args, function(r)
        ended(id, r)
    end)
end

local function cast()
    if tv then
        tv = nil
        tvcast_run({ "stop" })
        return
    end
    local path = mp.get_property("path")
    if not path or path:match("^%a[%w+.-]*://") then
        return mp.osd_message("Only local files can be cast")
    end

    mp.osd_message("Looking for TVs…", 5)
    tvcast_run({ "list" }, function(r)
        local tvs, names = {}, {}
        for url, name in trim(r.stdout):gmatch("([^\t\n]+)\t([^\n]+)") do
            table.insert(tvs, { url = url, name = name })
            table.insert(names, name)
        end
        if #tvs == 0 then
            return mp.osd_message(r.status == 0 and "No TVs found" or "TV search failed: " .. trim(r.stderr), 5)
        end
        mp.osd_message("")
        input.select({
            prompt = "Cast to:",
            items = names,
            submit = function(i)
                tv = tvs[i]
                start()
            end,
        })
    end)
end

mp.add_key_binding("C", "cast", cast)

mp.register_event("file-loaded", function()
    if tv then
        start()
    end
end)

mp.register_event("shutdown", function()
    tv = nil
    tvcast_run({ "stop" })
end)
