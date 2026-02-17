local inspect = require("inspect")

local data = {
    message = "Hello from Lua integration test!",
    status = "success"
}

print(inspect(data))
print("SUCCESS: Lua stack is working")
