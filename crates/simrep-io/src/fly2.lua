-- FLY2's pure constructors. Host access is confined to _emit and _read_lua.
for k,v in pairs(math) do _G[k] = v end
function math.random(m,n)
  local r=rand()
  if not m then return r end
  if not n then n=m;m=1 end
  assert(m<=n, 'empty random interval')
  return math.floor(m+(n-m+1)*r)
end
math.randomseed=_seed
simion.seed=_seed
mm,cm,m,um,nm=1,10,1000,0.001,0.000001
us,ns,ms,s=1,0.001,1000,1000000
eV,keV,MeV,amu=1,1000,1000000,1
function require(name) return _read_lua(name, true) end
function dofile(name) return _read_lua(name, false) end
simion.import = dofile
function print(...) end

local pi, sqrt, sin, cos = math.pi, math.sqrt, math.sin, math.cos
local function finite(x)
  return type(x)=='number' and x==x and x~=math.huge and x~=-math.huge
end
local function integer(x)
  assert(finite(x) and x >= 1 and x == math.floor(x), 'expected positive integer count')
  assert(x <= 10000000, 'FLY2 particle limit exceeded')
  return x
end
function vector(x,y,z)
  assert(finite(x) and finite(y) and finite(z), 'vector requires three finite components')
  return {x,y,z}
end
local function add(a,b) return vector(a[1]+b[1],a[2]+b[2],a[3]+b[3]) end
local function sub(a,b) return vector(a[1]-b[1],a[2]-b[2],a[3]-b[3]) end
local function mul(a,s) return vector(a[1]*s,a[2]*s,a[3]*s) end
local function norm(v)
  local r = sqrt(v[1]^2+v[2]^2+v[3]^2)
  if r == 0 then return vector(1,0,0) end
  return mul(v,1/r)
end
function rect3d_to_polar3d(x,y,z)
  return sqrt(x*x+y*y+z*z), math.deg(math.atan2(-z,x)), math.deg(math.atan2(y,sqrt(x*x+z*z)))
end
function polar3d_to_rect3d(r,az,el)
  az,el=math.rad(az),math.rad(el)
  return r*cos(el)*cos(az),r*sin(el),-r*cos(el)*sin(az)
end
function azimuth_rotate(a,x,y,z)
  a=math.rad(a)
  return x*cos(a)+z*sin(a),y,-x*sin(a)+z*cos(a)
end
function elevation_rotate(a,x,y,z)
  a=math.rad(a)
  return x*cos(a)-y*sin(a),x*sin(a)+y*cos(a),z
end
local function orient(v,axis)
  local _,az,el=rect3d_to_polar3d(unpack(axis))
  return vector(azimuth_rotate(az,elevation_rotate(el,unpack(v))))
end

local Source = {}
Source.__index = Source
local function source(n, get, random)
  return setmetatable({count=n,get=get,random=random}, Source)
end
function Source:iterator()
  local i=0
  return function()
    i=i+1
    if self.count and i>self.count then return nil end
    return self.get(i)
  end
end
function distribution(f)
  assert(type(f)=='function', 'distribution requires a function')
  return source(nil,f,true)
end
function sequence(t)
  assert(type(t)=='table' and #t>0, 'sequence requires a nonempty table')
  return source(#t,function(i) return t[(i-1)%#t+1] end)
end
function concatenated_sequence(t)
  local n=0
  for _,s in ipairs(t) do assert(getmetatable(s)==Source and s.count, 'finite sequence required'); n=n+s.count end
  return source(n,function(i)
    i=(i-1)%n+1
    for _,s in ipairs(t) do
      if i<=s.count then return s.get(i) end
      i=i-s.count
    end
  end)
end
function arithmetic_sequence(t)
  local first=t.first or 0
  local n=t.n
  local step=t.step
  if n then integer(n) end
  if not step then
    step=t.last and n and n>1 and (t.last-first)/(n-1) or 1
  end
  if not n and t.last then
    assert(step~=0, 'sequence step must be nonzero')
    n=integer(math.floor((t.last-first)/step+1+1e-10))
  end
  return source(n,function(i) return first+step*(i-1) end)
end
function line_sequence(t)
  local first=t.first or vector(0,0,0)
  local step=t.step
  local n=t.n and integer(t.n)
  if not step then
    assert(t.last and n, 'line_sequence requires step or last and n')
    step=n==1 and vector(0,0,0) or mul(sub(t.last,first),1/(n-1))
  end
  if not n and t.last then
    local d=sub(t.last,first)
    for a=1,3 do if step[a]~=0 then n=integer(math.floor(d[a]/step[a]+1+1e-10));break end end
  end
  return source(n,function(i) return add(first,mul(step,i-1)) end)
end
function uniform_distribution(t)
  local a,b=t.min or 0,t.max or 1
  return distribution(function() return a+(b-a)*rand() end)
end
function gaussian_distribution(t)
  local sigma=t.stdev or (t.fwhm and t.fwhm/sqrt(8*math.log(2))) or 1
  local mean=t.mean or 0
  return distribution(function()
    local u
    repeat u=rand() until u>0
    return mean+sigma*sqrt(-2*math.log(u))*cos(2*pi*rand())
  end)
end
normal_distribution=gaussian_distribution
function gaussian3d_distribution(t)
  local function component(v,a,default)
    if type(v)=='table' then return v[a] end
    return v or default
  end
  local g={}
  for a=1,3 do
    g[a]=gaussian_distribution {mean=component(t.mean,a,0),
      stdev=t.stdev and component(t.stdev,a,1),
      fwhm=t.fwhm and component(t.fwhm,a,1)}
  end
  return distribution(function() return vector(g[1].get(),g[2].get(),g[3].get()) end)
end
function line_distribution(t)
  local d=sub(t.last,t.first)
  return distribution(function() return add(t.first,mul(d,rand())) end)
end
local function circle(t,random,direction)
  assert(random or not t.fill, 'filled deterministic circle/cone sequences are not supported')
  local center=t.center or vector(0,0,0)
  local axis=t.normal or t.axis or vector(1,0,0)
  local n=not random and integer(t.n or 1)
  return source(n,function(i)
    local angle=2*pi*(random and rand() or (i-1)/n)
    if direction then
      local h=math.rad(t.half_angle or 0)
      local theta=t.fill and math.acos(1-rand()*(1-cos(h))) or h
      return orient(vector(cos(theta),sin(theta)*cos(angle),sin(theta)*sin(angle)),axis)
    end
    local r=t.radius or 1
    if t.fill then
      assert(random, 'filled circle_sequence is not supported')
      r=r*sqrt(rand())
    end
    return add(center,orient(vector(0,r*cos(angle),r*sin(angle)),axis))
  end,random)
end
function circle_sequence(t) return circle(t,false,false) end
function circle_distribution(t) return circle(t,true,false) end
function disc_distribution(t)
  return circle_distribution {center=t.center_position or t.center,
    normal=t.axis_direction or t.normal,radius=t.radius,fill=true}
end
function cone_direction_sequence(t) return circle(t,false,true) end
function cone_direction_distribution(t) return circle(t,true,true) end
function sphere_distribution(t)
  return distribution(function()
    local x=2*rand()-1
    local angle=2*pi*rand()
    local r=(t.radius or 1)*(t.fill and rand()^(1/3) or 1)
    return add(t.center or vector(0,0,0),vector(r*x,r*sqrt(1-x*x)*cos(angle),r*sqrt(1-x*x)*sin(angle)))
  end)
end

local definitions
function particles(t)
  assert(type(t)=='table', 'particles requires a table')
  assert(not definitions, 'particles must be called exactly once')
  definitions=t
end
local fields={'tob','mass','charge','x','y','z','az','el','ke','cwf','color','position','direction','speed','velocity','pupil','window'}
local known={n=true,format=true,pupil_position=true}
for _,k in ipairs(fields) do known[k]=true end
function standard_beam(t)
  assert(type(t)=='table', 'standard_beam requires a table')
  for k in pairs(t) do assert(known[k], 'unsupported standard_beam field '..tostring(k)) end
  if t.n then integer(t.n) end
  if t.pupil_position then t.pupil=t.pupil_position end
  t._standard_beam=true
  return t
end
standard_beam_define=standard_beam
local function sampled(value,index)
  if getmetatable(value)==Source then
    local i=value.count and ((index-1)%value.count+1) or index
    return value.get(i)
  end
  return value
end
function _finish_particles()
  assert(definitions, 'FLY2 did not call particles')
  local coordinates=definitions.coordinates or 0
  assert(finite(coordinates) and coordinates>=0 and coordinates==math.floor(coordinates), 'invalid coordinates mode')
  _check_coordinates(coordinates)
  for k in pairs(definitions) do assert(type(k)=='number' or k=='coordinates', 'unsupported particles field '..tostring(k)) end
  for _,beam in ipairs(definitions) do
    assert(beam._standard_beam, 'unsupported particle constructor')
    local count=beam.n
    if not count then
      count=1
      for _,key in ipairs(fields) do
        local s=beam[key]
        if getmetatable(s)==Source and s.count then count=count*s.count end
      end
    end
    integer(count)
    for i=1,count do
      local row={}
      local index=i-1
      for _,key in ipairs(fields) do
        local s=beam[key]
        if getmetatable(s)==Source and s.count then
          row[key]=sampled(s,index%s.count+1)
          index=math.floor(index/s.count)
        else row[key]=sampled(s,i) end
      end
      row.tob=row.tob or 0
      row.mass=row.mass or 0.00054857990946
      row.charge=row.charge or -1
      row.cwf=row.cwf or 1
      row.color=row.color or 0
      row.position=row.position or vector(row.x or 0,row.y or 0,row.z or 0)
      if row.window then
        row.direction=sub(row.position,row.window)
      elseif row.pupil then row.direction=sub(row.pupil,row.position)
      elseif row.velocity then row.direction=row.velocity
      elseif row.az or row.el then row.direction=vector(polar3d_to_rect3d(1,row.az or 0,row.el or 0))
      end
      row.direction=norm(row.direction or vector(1,0,0))
      local speed=row.speed
      if row.velocity then speed=sqrt(row.velocity[1]^2+row.velocity[2]^2+row.velocity[3]^2) end
      -- SIMION uses the 2010 CODATA amu and elementary charge constants.
      row.ke=row.ke or (speed and (0.5*1.660538921e-27/1.602176565e-19*1e6)*row.mass*speed^2) or 0.1
      assert(finite(row.color), 'color must be finite')
      row.color=(row.color>=0 and math.floor(row.color) or math.ceil(row.color))%16
      _emit(coordinates,row)
    end
  end
end
