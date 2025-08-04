My alternative to GNU Stow. This only compiles on macOS.

This allows having Lua files in the dotty directory.
To link a Lua file to a dot file, append `.lua` to the file name.

Those Lua files can return:

- `true` to indicate that the file should be linked
- `false` to indicate that the file should not be linked
- a table with 1 key `rename_to` to indicate that the file should be linked to a different name

# Example Lua file

This checks if `~/.quinscape` exists.

```lua
local home = assert(os.getenv("HOME"), "HOME environment variable must be set")
local path = home .. "/.quinscape"

local function file_exists(p)
	local f = io.open(p, "r")
	if f then
		f:close()
		return true
	end
	return false
end

return file_exists(path)
```
