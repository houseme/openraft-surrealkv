-- benchmark.lua
-- wrk -t12 -c400 -d30s -s benchmark.lua http://127.0.0.1:8080

math.randomseed(os.time())

local requests = {
    [1] = function()
        return wrk.format("POST", "/write",
            {["Content-Type"] = "application/json"},
            '{"Set":{"key":"key' .. math.random(1,100000) .. '","value":"value' .. os.clock() .. '"}}')
    end,
    [2] = function()
        return wrk.format("POST", "/read",
            {["Content-Type"] = "application/json"},
            '"key' .. math.random(1,100000) .. '"')
    end
}

request = function()
    local r = requests[math.random(1, #requests)]
    return r()
end

response = function(status, headers, body)
    if status ~= 200 then
        io.write("Error: " .. status .. " " .. body .. "\n")
    end
end