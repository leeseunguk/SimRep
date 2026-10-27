import math

import pytest
from simrep import uncertainty


def test_celik_2008_worked_example():
    result = uncertainty.observed_order(
        [n ** -0.5 for n in (18000, 8000, 4500)], [6.063, 5.972, 5.863])
    assert result["p"] == pytest.approx(1.53, abs=0.005)
    assert result["extrapolated"] == pytest.approx(6.1685, abs=0.0001)
    assert result["gci_fine"] * 100 == pytest.approx(2.2, abs=0.05)


def test_least_squares_and_noisy_sequence():
    h = [0.125, 0.25, 0.5, 1.0]
    values = [2 + 0.7 * x ** 1.5 for x in h]
    result = uncertainty.grid_study(h, values)
    assert result["p"] == pytest.approx(1.5, abs=1e-8)
    assert result["extrapolated"] == pytest.approx(2, abs=1e-10)
    assert result["u_num"] == pytest.approx(1.25 * (values[0] - 2), abs=1e-10)
    noisy = [v + (0.2 if i % 2 == 0 else -0.2) for i, v in enumerate(values)]
    result = uncertainty.grid_study(h, noisy)
    assert result["p"] is None
    assert result["safety_factor"] == 3
    assert result["u_num"] == pytest.approx(1.5945, abs=0.001)


def test_validation_and_invalid_inputs():
    result = uncertainty.validation(1, 2, 1.5, 0.5, 0.25)
    assert result["E"] == -1
    assert result["u_val"] == math.hypot(1.5, 0.5, 0.25)
    for h, values in [([0, 1, 2], [1, 2, 3]),
                      ([1, 2, 3], [1, math.nan, 3]),
                      ([1, 2], [1, 2, 3])]:
        with pytest.raises(ValueError):
            uncertainty.observed_order(h, values)
    with pytest.raises(ValueError):
        uncertainty.validation(1, 2, -1)
