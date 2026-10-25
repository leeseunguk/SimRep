-- High precision readout only. Copy beside an oracle IOB as einzel.lua.
-- If testplanelib.lua is present, reproduce the test_plane example's planes.
simion.workbench_program()
sim_segment_global = 1

function segment.initialize()
  print(string.format('START,%d,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g',
    ion_number, ion_time_of_birth, ion_mass, ion_charge,
    ion_px_mm, ion_py_mm, ion_pz_mm, ion_vx_mm, ion_vy_mm, ion_vz_mm, ion_ke))
end

function segment.terminate()
  print(string.format('FINAL,%d,%.17g,%.17g,%.17g,%.17g,%.17g',
    ion_number, ion_time_of_flight, ion_px_mm, ion_py_mm, ion_pz_mm, ion_ke))
end

local file = io.open('testplanelib.lua', 'r')
if file then
  file:close()
  local TP = simion.import 'testplanelib.lua'
  local first = TP(20,0,0, 1,0,0)
  local second = TP(40,0,0, 1,-1,0)
  local third = TP(80,0,0, 1,1,0, function()
    print(string.format('PLANE3,%d,%.17g,%.17g,%.17g,%.17g,%.17g',
      ion_number, ion_time_of_flight, ion_px_mm, ion_py_mm, ion_pz_mm, ion_ke))
  end)
  function segment.tstep_adjust()
    first.tstep_adjust()
    second.tstep_adjust()
    third.tstep_adjust()
  end
  function segment.other_actions()
    first.other_actions()
    second.other_actions()
    third.other_actions()
  end
end
