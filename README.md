My alternative to GNU Stow. This only compiles on macOS.

This allows having Lua files in the dotty directory.
To link a Lua file to a dot file, append `.lua` to the file name.

Those Lua files can return:

- `true` to indicate that the file should be linked
- `false` to indicate that the file should not be linked
- a table with 1 or 2 keys:
    - `rename_to`: a string to indicate that the file should be linked or written to a different file name.
    - `transform`: a function that receives the original file content as a string and must return a new string. If provided, `dotty` will write a new file with the transformed content instead of creating a symlink.

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

# Example Lua file with `transform`

This example reads a source file, replaces a placeholder email address with one from an environment variable, and writes the result to `~/.gitconfig`.

**`gitconfig-template` (in dotty root):**
```ini
[user]
    email = YOUR_EMAIL_HERE
```

**`gitconfig-template.lua` (in dotty root):**
```lua
return {
  -- The target file in home will be ~/.gitconfig
  rename_to = ".gitconfig",

  transform = function(content)
    -- Fetches your real email from an environment variable
    local email = assert(os.getenv("EMAIL"), "EMAIL environment variable must be set")
    -- Replaces the placeholder with the real email
    return string.gsub(content, "YOUR_EMAIL_HERE", email)
  end
}
```

When `dotty` runs, it will create a file at `~/.gitconfig` with the email address replaced, instead of creating a symlink.
