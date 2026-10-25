-- Diagnose SIMION RNG initialization without flying particles.
for round=1,3 do
  seed(237)
  local values={}
  for i=1,5 do values[i]=string.format('%.17g',rand()) end
  print('SEQUENCE',round,table.concat(values,','))
end
