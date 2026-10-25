import numpy as np
import pytest

import simrep


def test_fly2_uses_the_in_memory_pa_and_placement(tmp_path):
    pa = simrep.PA(np.full((1, 7, 7), 5.0), np.zeros((1, 7, 7), bool),
                   {"spacing_mm": [0.5, 1.0, 1.0]})
    pa.electrodes[0, 3, 3] = True
    path = tmp_path / "context.fly2"
    path.write_text("""
local instance = simion.wb.instances[1]
assert(instance.pa:electrode(3,3,0))
particles {coordinates=1, standard_beam {
  mass=39, charge=1, position=vector(2,3,0), direction=vector(1,0,0),
  ke=instance:potential_wc(10,20,30)
}}
""")
    with pytest.raises(ValueError, match="missing workbench instance context"):
        simrep.read_particles(path)
    result = simrep.fly(pa, path, device="cpu", origin_mm=[10, 20, 30],
                        axes=[[0, 1, 0], [-1, 0, 0], [0, 0, 1]], scale=2,
                        duration_us=0.001, trajectories=True, allow_time_limit=True)
    np.testing.assert_allclose(result.trajectories[0][0, 1:4], [4, 22, 30],
                               rtol=0, atol=1e-12)
    np.testing.assert_allclose(result.arrays["ke_ev"], [5], rtol=0, atol=1e-12)


def test_fly2_field_query_includes_initial_voltage_expressions(tmp_path):
    values = np.zeros((1, 7, 9))
    mask = np.zeros_like(values, dtype=bool)
    mask[:, :, 0] = mask[:, :, -1] = True
    values[:, :, 0], values[:, :, -1] = 1, 2
    field = simrep.refine(simrep.PA(values, mask), adjustable=True)
    path = tmp_path / "potential.fly2"
    path.write_text("""particles {standard_beam {
mass=39,charge=0,position=vector(4,3,0),direction=vector(1,0,0),
ke=simion.wb:epotential(4,3,0)}}""")
    static = simrep.fly(field, path, voltages={1: 0, 2: 7},
                        device="cpu", duration_us=0.001, allow_time_limit=True)
    dynamic = simrep.fly(field, path, voltage_expr={2: "v"}, parameters={"v": 7},
                         device="cpu", duration_us=0.001, allow_time_limit=True)
    np.testing.assert_allclose(dynamic.arrays["ke_ev"], static.arrays["ke_ev"],
                               rtol=0, atol=1e-12)
    np.testing.assert_allclose(dynamic.arrays["ke_ev"], [3.5], rtol=0, atol=1e-9)
