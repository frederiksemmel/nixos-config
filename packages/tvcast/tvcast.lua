-- `C` casts the current file to a DLNA TV picked from a menu. While casting,
-- mpv stays paused and its playback keys drive the TV; `C` again ends the
-- cast and leaves mpv at the TV's position.

local input = require("mp.input")

local tvcast = "@tvcast@"
local casting = false

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

local function start(tv)
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

    mp.set_property_bool("pause", true)
    casting = true
    set_remote(true)
    mp.osd_message("Casting to " .. tv.name)
    tvcast_run(args, function(r)
        casting = false
        set_remote(false)
        if r.status == 0 then
            mp.osd_message("Cast ended")
        else
            mp.osd_message("Cast failed: " .. trim(r.stderr), 5)
        end
    end)
end

local function stop()
    local pos = tonumber(trim(tvcast_run({ "position" }).stdout))
    tvcast_run({ "stop" })
    if pos then
        mp.commandv("seek", pos, "absolute")
    end
end

local function cast()
    if casting then
        return stop()
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
                start(tvs[i])
            end,
        })
    end)
end

mp.add_key_binding("C", "cast", cast)

mp.register_event("shutdown", function()
    if casting then
        tvcast_run({ "stop" })
    end
end)
