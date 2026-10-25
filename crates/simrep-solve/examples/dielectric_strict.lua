-- Run in the writable SIMION dielectric example copy. Originals are not saved.
-- simion.exe --nogui --noprompt lua <path-to-this-file>
for _, name in ipairs({'plates_2dp', 'sphere_2dc', 'sphere_3dp'}) do
  simion.pas:close()
  local pa = simion.pas:open(name .. '.pa')
  local epsilon = simion.pas:open(name .. '-dielectric.pa')
  if name ~= 'plates_2dp' then
    for x,y,z in epsilon:points() do
      epsilon:potential(x,y,z, epsilon:potential(x,y,z) == 1 and 1 or 10)
    end
  end
  for repetition=1,3 do
    for x,y,z in pa:points() do
      pa:potential(x,y,z, x == 0 and 100 or 0)
    end
    print('DIELECTRIC_STRICT_BEGIN,' .. name .. ',' .. repetition)
    pa:refine{permittivity=epsilon, convergence=1e-7}
  end
  local center = name == 'plates_2dp' and 50 or 100
  local field = (pa:potential(center-1,0,0) - pa:potential(center+1,0,0)) / (2*pa.dx_mm)
  print(string.format('DIELECTRIC_STRICT_FIELD,%s,%.17g', name, field))
  pa:save(name .. '.simrep-strict.pa')
end
