local home = assert(os.getenv("HOME"), "HOME environment variable must be set")
local path = home .. "/gnu stow reimplementation.md"

local function file_exists(p)
	local f = io.open(p, "r")
	if f then
		f:close()
		return true
	end
	return false
end

if file_exists(path) then
	return path
else
	return "not found: " .. path
end
