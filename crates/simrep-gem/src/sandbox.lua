-- Pure standard libraries and GEM conveniences. No host file or process handles.
for k,v in pairs(math) do _G[k] = v end
inf = math.huge
function rect3d_to_polar3d(x,y,z)
  local r = math.sqrt(x*x+y*y+z*z)
  return r, math.deg(math.atan2(-z,x)), math.deg(math.atan2(y,math.sqrt(x*x+z*z)))
end
function polar3d_to_rect3d(r,az,el)
  az,el = math.rad(az),math.rad(el)
  return r*math.cos(el)*math.cos(az),r*math.sin(el),-r*math.cos(el)*math.sin(az)
end
local function length(v)
  return setmetatable({__mm=v}, getmetatable(mm))
end
local function number(v) return type(v)=='table' and v.__mm or v end
mm = setmetatable({__mm=1}, {
  __mul=function(a,b) return length(number(a)*number(b)) end,
  __div=function(a,b) return length(number(a)/number(b)) end,
  __add=function(a,b) return length(number(a)+number(b)) end,
  __sub=function(a,b) return length(number(a)-number(b)) end,
  __unm=function(a) return length(-number(a)) end
})
function _arguments(body)
  local t = {}
  body(function(...)
    for i=1,select('#',...) do t[#t+1] = select(i,...) end
  end)
  return unpack(t)
end
-- Parameter-file writes in installed examples are confined to this compilation.
local files = {}
io = {}
function io.open(name, mode)
  if mode ~= 'w' and mode ~= 'wb' then return nil, 'GEM sandbox: only virtual text writes are allowed' end
  local file = {text='', closed=false}
  files[name] = file
  function file:write(...)
    assert(not self.closed, 'file is closed')
    for i=1,select('#',...) do self.text = self.text .. tostring(select(i,...)) end
    return self
  end
  function file:close() self.closed=true return true end
  return file
end
