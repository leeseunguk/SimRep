-- Endpoint oracle, not the shipped repulsion1 potential diagnostic.
simion.workbench_program()
sim_segment_global = 1
adjustable beam_max_dt = 1e99

function segment.initialize_run()
  sim_grouped = 1
  sim_relativity = 0
  sim_repulsion = 'beam'
end

function segment.initialize()
  print(string.format('BEAM_START,%d,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g',
    ion_number, ion_time_of_birth, ion_mass, ion_charge, ion_px_mm, ion_py_mm,
    ion_pz_mm, ion_vx_mm, ion_vy_mm, ion_vz_mm, ion_ke, ion_cwf))
end

function segment.tstep_adjust()
  ion_time_step = math.min(ion_time_step, beam_max_dt)
end

function segment.terminate()
  print(string.format('BEAM_FINAL,%d,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g,%.17g',
    ion_number, ion_time_of_flight, ion_px_mm, ion_py_mm, ion_pz_mm,
    ion_vx_mm, ion_vy_mm, ion_vz_mm, ion_ke))
end
