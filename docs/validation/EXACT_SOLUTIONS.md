# Exact-solution verification catalog

Generated from `exact_solutions.json` by `python3 tools/exact_solutions.py --print`.
Check the rendering with `--check`; execute every linked CPU public-API test with `--run`.
Computed values, numerical uncertainty, and MISS/OPEN findings are in [GATES.md](GATES.md).

Equations use SI unless the symbol explicitly states otherwise. SimRep converts to mm, us, V, gauss, amu, elementary charges, and eV. A kinetic energy per charge in joules/coulomb is a voltage; for singly charged ions its numerical value equals the energy in eV.

Each case names executable public Rust CPU API tests, not a second implementation of SimRep. Formula provenance is explicitly derived here: no unchecked textbook page or equation is claimed. Spatial or temporal three-level Celik GCI uses a safety factor of 1.25 where the observable has identifiable monotone convergence. Roundoff-limited or finite-ray cases remain OPEN without an invented uncertainty. Test tolerances and approximation envelopes are not numerical uncertainties. MISS is reported, not hidden by widening a tolerance.

The inventory follows the formula-to-executable-case approach of [Iteration-CFD's V&V plan](https://raw.githubusercontent.com/using76/Iteration-CFD/main/docs/19-vv-plan.md). SimRep's implemented equations are specified in [docs/spec/equations.md](../spec/equations.md).

## Case index

| Case | Physics | Gates |
| --- | --- | --- |
| [parallel-plate-deflection](#parallel-plate-deflection) | Nonrelativistic transverse electrostatic deflection | `exact-plate-deflection` |
| [parallel-plate-tof](#parallel-plate-tof) | Uniform longitudinal electrostatic acceleration | `comparison-parallel-plate-tof` |
| [spherical-analyzer](#spherical-analyzer) | Concentric spherical electrostatic analyzer pass energy and 180-degree first-order focusing | `exact-spherical-passband`, `exact-spherical-focus` |
| [cylindrical-analyzer](#cylindrical-analyzer) | Coaxial cylindrical analyzer pass energy and 127.279-degree first-order radial focusing | `exact-cylindrical-passband`, `exact-cylindrical-focus` |
| [toroidal-small-gap](#toroidal-small-gap) | Asymptotic voltage-to-pass-energy calibration for toroidal surfaces | `exact-toroidal-small-gap` |
| [relativistic-cyclotron](#relativistic-cyclotron) | Relativistic uniform-magnetic-field radius and period | `exact-relativistic-cyclotron-radius`, `exact-relativistic-cyclotron-period` |
| [exb-drift](#exb-drift) | Uniform crossed-field gyro-averaged drift | `exact-exb-drift` |
| [wien-filter](#wien-filter) | Crossed electric/magnetic force cancellation and velocity selection | `exact-wien-balance` |
| [laplace-coax](#laplace-coax) | Potential between infinite coaxial cylindrical conductors | `exact-laplace-coax`, `mms-cut-cylinder-potential`, `mms-cut-cylinder-field` |
| [laplace-spheres](#laplace-spheres) | Potential between concentric spherical conductors | `exact-laplace-spheres`, `mms-cut-sphere-potential`, `mms-cut-sphere-field` |
| [coulomb-point-charge](#coulomb-point-charge) | Unsoftened electrostatic point-charge field | `exact-coulomb-point-charge` |
| [coulomb-two-body](#coulomb-two-body) | Cold symmetric repulsive two-body Coulomb expansion | `exact-coulomb-two-body` |
| [child-langmuir](#child-langmuir) | Planar cold space-charge-limited diode solution | `exact-child-langmuir-current` |

## parallel-plate-deflection

Nonrelativistic transverse electrostatic deflection

$$
E_y=U/d,\quad t=L/v_x
$$

$$
y(L)=y_0+v_{y0}L/v_x+\frac{qU L^2}{2mdv_x^2}
$$

| Symbols | Units | Meaning |
| --- | --- | --- |
| `E_y` | V/m | uniform transverse electric field |
| `U` | V | signed voltage producing positive transverse field |
| `d,L,y,y_0` | m | plate gap, active length, transverse position and launch position |
| `v_x,v_{y0}` | m/s | constant longitudinal and initial transverse velocities |
| `q,m,t` | C, kg, s | particle charge, mass and flight time |

**Validity:** Infinite plates, uniform static field with no fringe field; nonrelativistic; no space charge or collisions; trajectory remains within the gap.

**Provenance:** derived here.

1. Newton-Lorentz gives x''=0 and y''=qU/(md).
2. Integrate twice with the launch position and velocity.
3. Set x-x0=L=v_x t and substitute t=L/v_x.

**Executable checks:**

- [`exact_solutions.rs::parallel_plate_deflection`](../../crates/simrep-fly/tests/exact_solutions.rs): `cargo test --release -p simrep-fly --test exact_solutions parallel_plate_deflection -- --exact --nocapture`

**Band and uncertainty:** 1e-10 mm absolute deflection; 1e-12 us transit time; uniform polynomial trajectory, u_num not identifiable.

## parallel-plate-tof

Uniform longitudinal electrostatic acceleration

$$
a=qU/(md),\quad L=v_0t+\tfrac12 at^2
$$

$$
t=\frac{2L}{v_0+\sqrt{v_0^2+2aL}},\qquad t_{v_0=0}=\sqrt{2L/a}
$$

| Symbols | Units | Meaning |
| --- | --- | --- |
| `a` | m/s^2 | constant acceleration toward detector |
| `q,m,U,d` | C, kg, V, m | charge, mass, voltage and plate gap |
| `L,v_0,t` | m, m/s, s | flight distance, initial longitudinal speed and elapsed TOF |

**Validity:** Same idealizations as parallel-plate deflection; positive physical root with detector reachable. Existing test launches from rest and also verifies that birth time is excluded from TOF.

**Provenance:** derived here.

1. Integrate x''=a to L=v0 t+a t^2/2.
2. Take the positive quadratic root and rationalize it.
3. For v0=0 obtain sqrt(2L/a); absolute arrival time is birth time plus t.

**Executable checks:**

- [`accuracy.rs::parallel_plate_tof_and_birth_time`](../../crates/simrep-fly/tests/accuracy.rs): `cargo test --release -p simrep-fly --test accuracy parallel_plate_tof_and_birth_time -- --exact --nocapture`

**Band and uncertainty:** Existing relative TOF tolerance 1e-12; reused, not duplicated; roundoff-limited OPEN gate.

## spherical-analyzer

Concentric spherical electrostatic analyzer pass energy and 180-degree first-order focusing

$$
\Phi(r)=A-B/r,\quad B=\frac{U R_1R_2}{R_2-R_1}
$$

$$
\frac{K_0}{q}=\frac{B}{2R_0}=\frac{U R_1R_2}{R_2^2-R_1^2},\quad R_0=(R_1+R_2)/2
$$

$$
\delta r''(\theta)+\delta r(\theta)=0,\quad \delta r(\theta)=R_0\alpha\sin\theta+O(\alpha^2),\quad \theta_f=\pi
$$

| Symbols | Units | Meaning |
| --- | --- | --- |
| `Phi,A,U` | V | potential, gauge constant, outer-minus-inner voltage |
| `B` | V m | inverse-radius potential coefficient |
| `R_1,R_2,R_0,r,delta r` | m | inner, outer, mean, trajectory radius and radial perturbation |
| `K_0,q` | J, C | central kinetic energy and positive ion charge |
| `theta,theta_f,alpha` | rad | bend angle, first-order focus angle and small launch-angle error |

**Validity:** Nonrelativistic positive ions, attractive inverse-square electric force, ideal concentric spherical surfaces with no fringes. Stated rational passband assumes arithmetic mean orbit radius, not an arbitrary radius. The focus is first-order angular focusing, not zero finite-angle aberration or a claim of finite aperture energy bandwidth.

**Provenance:** derived here.

1. Spherical Laplace gives Phi=A-B/r; impose Phi(R2)-Phi(R1)=U to obtain B.
2. The circular-orbit equation m v^2/R0=qB/R0^2 gives K0/q=B/(2R0).
3. For radial force proportional to r^(-s), linearize the Binet equation in u=1/r: delta u''+(3-s)delta u=0.
4. Here s=2, so a launch-angle perturbation is proportional to sin(theta); its first angular focus is pi.

**Executable checks:**

- [`exact_solutions.rs::spherical_and_cylindrical_passband_and_first_order_focus`](../../crates/simrep-fly/tests/exact_solutions.rs): `cargo test --release -p simrep-fly --test exact_solutions spherical_and_cylindrical_passband_and_first_order_focus -- --exact --nocapture`

**Band and uncertainty:** Central exit radius relative tolerance 5e-5 with 128/256/512-step GCI; normalized odd angular response below 2e-5 per rad at alpha=1e-3 rad and 4096 steps, no uncertainty assigned to paraxial limit.

## cylindrical-analyzer

Coaxial cylindrical analyzer pass energy and 127.279-degree first-order radial focusing

$$
\Phi(r)=A+\frac{U\ln(r/R_1)}{\ln(R_2/R_1)},\quad E_r=-\frac{U}{r\ln(R_2/R_1)}
$$

$$
\frac{K_0}{q}=\frac{U}{2\ln(R_2/R_1)}
$$

$$
\delta r(\theta)=\frac{R_0\alpha}{\sqrt2}\sin(\sqrt2\theta)+O(\alpha^2),\quad\theta_f=\frac{\pi}{\sqrt2}=127.2792206\ldots\,^\circ
$$

| Symbols | Units | Meaning |
| --- | --- | --- |
| `Phi,A,U` | V | potential, inner gauge and outer-minus-inner voltage |
| `E_r` | V/m | radial electric field |
| `R_1,R_2,R_0,r,delta r` | m | electrode, reference, trajectory radii and perturbation |
| `K_0,q` | J, C | central kinetic energy and positive charge |
| `alpha,theta,theta_f` | rad | small launch-angle error, bend angle and radial focus |

**Validity:** Infinite coaxial cylinders; nonrelativistic positive ions; no end/fringe field; first-order radial focusing only. An ideal cylindrical analyzer has no analogous axial focusing.

**Provenance:** derived here.

1. Integrate cylindrical Laplace (r Phi')'=0 with voltage difference U.
2. Balance m v^2/r=qU/[r ln(R2/R1)] to find the radius-independent pass energy.
3. Linearize Binet as in the spherical case, now s=1; delta u''+2 delta u=0.
4. The first zero after launch is theta=pi/sqrt(2), commonly rounded to 127 degrees.

**Executable checks:**

- [`exact_solutions.rs::spherical_and_cylindrical_passband_and_first_order_focus`](../../crates/simrep-fly/tests/exact_solutions.rs): `cargo test --release -p simrep-fly --test exact_solutions spherical_and_cylindrical_passband_and_first_order_focus -- --exact --nocapture`

**Band and uncertainty:** Same central-ray and angular-response bands as spherical analyzer; this is a pass-energy check, not a finite-aperture spectral bandwidth. Circular-ray step differences below 64 binary64 eps times the observable scale are roundoff-limited; no spurious GCI is assigned.

## toroidal-small-gap

Asymptotic voltage-to-pass-energy calibration for toroidal surfaces

$$
E_n\sim U/d,\quad K_0/q\sim U R_0/(2d)
$$

$$
k\equiv\frac{U}{K_0/q}\sim\frac{2d}{R_0}
$$

| Symbols | Units | Meaning |
| --- | --- | --- |
| `E_n,U` | V/m, V | gap-normal electric field and electrode voltage difference |
| `d,R_0,a` | m | full gap, equatorial reference orbit radius and poloidal curvature radius |
| `K_0,q,k` | J, C, 1 | pass energy, charge and explicitly defined calibration constant |

**Validity:** Asymptotic, not an exact finite-gap torus solution: d/R0 and d/a must be small; symmetric gap about reference surface, no fringes. The convention k=U/(K/q) matters; some analyzer literature uses its reciprocal. Test uses circular torus minor radii 0.9 and 1.1 mm, major radius 4 mm, R0=5 mm, d=0.2 mm.

**Provenance:** derived here.

1. In normal coordinates across a smooth thin gap, the leading Laplace operator is d^2/dn^2, hence Phi is linear to leading order and En~U/d.
2. Equatorial circular-orbit balance gives 2K0/R0=q En.
3. Define k=U/(K0/q) and substitute En. Curvature corrections remain at finite gap and are not numerical uncertainty.

**Executable checks:**

- [`exact_solutions.rs::toroidal_small_gap_calibration`](../../crates/simrep-fly/tests/exact_solutions.rs): `cargo test --release -p simrep-fly --test exact_solutions toroidal_small_gap_calibration -- --exact --nocapture`

**Band and uncertainty:** 2% relative approximation envelope; independent axisymmetric cut-cell Laplace solves on 49/97/193 grids; GCI estimates discretization only. A MISS against the leading asymptote is a finite-gap finding, not by itself a solver defect.

## relativistic-cyclotron

Relativistic uniform-magnetic-field radius and period

$$
\gamma=1/\sqrt{1-v^2/c^2}=1+K/(mc^2)
$$

$$
\omega_c=|q|B/(\gamma m),\quad r_c=p_\perp/(|q|B)=\gamma m v_\perp/(|q|B),\quad T_c=2\pi\gamma m/(|q|B)
$$

| Symbols | Units | Meaning |
| --- | --- | --- |
| `gamma` | 1 | Lorentz factor |
| `v,v_perp,c` | m/s | total and perpendicular speed and speed of light |
| `K,m,q,B,p_perp` | J, kg, C, T, kg m/s | kinetic energy, rest mass, charge, field and transverse momentum |
| `omega_c,r_c,T_c` | rad/s, m, s | cyclotron angular frequency, radius and period |

**Validity:** Uniform static B, no E, collisions, radiation reaction or collective effects; test has perpendicular launch and K=1 GeV for mass 1 amu. Gamma is constant because magnetic force does no work.

**Provenance:** derived here.

1. Start with d(gamma m v)/dt=q v cross B.
2. Dot with v: kinetic energy and gamma are constant.
3. The perpendicular velocity rotates at |q|B/(gamma m); integrate its rotation to obtain radius and period.
4. The test infers the period from the quarter-orbit position angle, not by substituting the requested duration as a measured period.

**Executable checks:**

- [`exact_solutions.rs::relativistic_cyclotron_radius_and_period`](../../crates/simrep-fly/tests/exact_solutions.rs): `cargo test --release -p simrep-fly --test exact_solutions relativistic_cyclotron_radius_and_period -- --exact --nocapture`

**Band and uncertainty:** Relative radius and inferred-period tolerances 2e-6; relativistic Boris 128/256/512 steps per quarter orbit; three-step GCI. Existing nonrelativistic 1000-orbit energy test is not duplicated.

## exb-drift

Uniform crossed-field gyro-averaged drift

$$
\mathbf v_D=\frac{\mathbf E\times\mathbf B}{B^2},\quad\mathbf E\cdot\mathbf B=0
$$

$$
E=E_x\hat x,\ B=B_z\hat z:\quad x(t)=\frac{E_x}{B_z\omega_c}(1-\cos\omega_ct),\quad y(t)=-\frac{E_x}{B_z}t+\frac{E_x}{B_z\omega_c}\sin\omega_ct
$$

| Symbols | Units | Meaning |
| --- | --- | --- |
| `E,B` | V/m, T | uniform orthogonal fields |
| `v_D` | m/s | gyro-averaged drift velocity |
| `omega_c,t,x,y` | rad/s, s, m, m | signed nonrelativistic frequency qB/m, time and positions |

**Validity:** Nonrelativistic uniform perpendicular fields; the displayed trajectory starts at rest at the origin. Drift is independent of mass and charge, but gyromotion is not. SimRep conversion is vD[mm/us]=1e4 (E[V/mm] cross B[G])/|B[G]|^2.

**Provenance:** derived here.

1. Write v=vD+w and choose E+vD cross B=0.
2. The remaining equation is pure gyromotion for w.
3. Integrate the rest-launch initial condition; after one physical gyroperiod the sine/cosine terms cancel and displacement/T equals vD.

**Executable checks:**

- [`exact_solutions.rs::crossed_field_drift_and_wien_balance`](../../crates/simrep-fly/tests/exact_solutions.rs): `cargo test --release -p simrep-fly --test exact_solutions crossed_field_drift_and_wien_balance -- --exact --nocapture`

**Band and uncertainty:** Mean drift relative tolerance 2e-5; 128/256/512 Boris steps per physical gyroperiod with temporal GCI.

## wien-filter

Crossed electric/magnetic force cancellation and velocity selection

$$
\mathbf E+\mathbf v\times\mathbf B=0,\quad v_\perp=E/B
$$

| Symbols | Units | Meaning |
| --- | --- | --- |
| `E,B,v_perp` | V/m, T, m/s | orthogonal electric and magnetic fields and selected transverse speed |

**Validity:** Uniform perpendicular fields; direction must satisfy the vector equation, not just the speed ratio. Any parallel-to-B velocity is unconstrained. Force cancellation is also relativistically valid, but this test is nonrelativistic.

**Provenance:** derived here.

1. Set the Lorentz force q(E+v cross B) to zero.
2. For perpendicular fields choose v=(E cross B)/B^2, obtaining selected speed E/B.
3. The test also detunes the incident speed by 10% and checks the analytic transverse displacement, so a field implementation returning zero cannot pass.

**Executable checks:**

- [`exact_solutions.rs::crossed_field_drift_and_wien_balance`](../../crates/simrep-fly/tests/exact_solutions.rs): `cargo test --release -p simrep-fly --test exact_solutions crossed_field_drift_and_wien_balance -- --exact --nocapture`

**Band and uncertainty:** Selected-ray transverse displacement and longitudinal position below 1e-12 mm absolute; detuned-ray displacement relative tolerance 1e-5; balanced solution is roundoff-limited OPEN.

## laplace-coax

Potential between infinite coaxial cylindrical conductors

$$
\Phi(r)=V_2+(V_1-V_2)\frac{\ln(R_2/r)}{\ln(R_2/R_1)},\quad E_r=\frac{V_1-V_2}{r\ln(R_2/R_1)}
$$

| Symbols | Units | Meaning |
| --- | --- | --- |
| `Phi,V_1,V_2` | V | vacuum potential and conductor voltages |
| `r,R_1,R_2` | m | radial coordinate and conductor radii |
| `E_r` | V/m | outward radial field |

**Validity:** R1<r<R2, infinite coaxial cylinders, vacuum, no volume charge. Existing Cartesian cut-cell test uses four grid phases and exact conductor surfaces; no staircase-boundary substitution.

**Provenance:** derived here.

1. Cylindrical symmetry reduces Laplace to (r Phi')'=0.
2. Integrate to Phi=A+B ln r and impose both conductor voltages.
3. Differentiate with E=-grad Phi for the field.

**Executable checks:**

- [`mms.rs::exact_surface_mms_orders_and_shifted_reference_frame`](../../crates/simrep-solve/tests/mms.rs): `cargo test --release -p simrep-solve --test mms exact_surface_mms_orders_and_shifted_reference_frame -- --exact --nocapture`

**Band and uncertainty:** Reuse 17/33/65 grids and worst of four phases; existing max potential error <6h^2 V and near-surface field error <20h V/mm; GCI of maximum potential-error norm. Order gates remain OPEN.

## laplace-spheres

Potential between concentric spherical conductors

$$
\Phi(r)=V_2+(V_1-V_2)\frac{1/r-1/R_2}{1/R_1-1/R_2},\quad E_r=\frac{V_1-V_2}{r^2(1/R_1-1/R_2)}
$$

| Symbols | Units | Meaning |
| --- | --- | --- |
| `Phi,V_1,V_2` | V | vacuum potential and conductor voltages |
| `r,R_1,R_2` | m | radial coordinate and sphere radii |
| `E_r` | V/m | outward radial field |

**Validity:** R1<r<R2, concentric spherical conductors, vacuum, no volume charge; exact surfaces and four grid phases in the existing 3D solver test.

**Provenance:** derived here.

1. Spherical symmetry reduces Laplace to (r^2 Phi')'=0.
2. Integrate to Phi=A+B/r, apply the two Dirichlet voltages.
3. Differentiate to obtain the inverse-square radial field.

**Executable checks:**

- [`mms.rs::exact_surface_mms_orders_and_shifted_reference_frame`](../../crates/simrep-solve/tests/mms.rs): `cargo test --release -p simrep-solve --test mms exact_surface_mms_orders_and_shifted_reference_frame -- --exact --nocapture`

**Band and uncertainty:** Same existing grids, phase sweep and absolute envelopes as coaxial case; GCI of maximum potential-error norm; any pre-asymptotic order miss remains a finding.

## coulomb-point-charge

Unsoftened electrostatic point-charge field

$$
\Phi(\mathbf r)=\frac{Q}{4\pi\epsilon_0 r},\quad \mathbf E(\mathbf r)=\frac{Q\mathbf r}{4\pi\epsilon_0 r^3}
$$

| Symbols | Units | Meaning |
| --- | --- | --- |
| `Q,epsilon_0` | C, F/m | source charge and vacuum permittivity |
| `r` | m | source-to-observer displacement and its magnitude |
| `Phi,E` | V, V/m | potential and electric field |

**Validity:** Vacuum point charge at nonzero separation; no softening, self-force or conductor images. Existing test checks the DirectCharge field, not a potential-output API, and also checks represented-charge weights and equal/opposite fields.

**Provenance:** derived here.

1. Gauss's law on a sphere gives 4 pi r^2 Er=Q/epsilon0.
2. Choose Phi(infinity)=0 and integrate -Er to obtain the potential.
3. Use the displacement direction to write the vector field.

**Executable checks:**

- [`space_charge.rs::represented_charge_weights_and_coulomb_units`](../../crates/simrep-fly/tests/space_charge.rs): `cargo test --release -p simrep-fly --test space_charge represented_charge_weights_and_coulomb_units -- --exact --nocapture`

**Band and uncertainty:** Reuse existing absolute field tolerance 1e-16 V/mm at 2 mm separation for Q=1e-15 C; direct algebra has no identifiable grid uncertainty.

## coulomb-two-body

Cold symmetric repulsive two-body Coulomb expansion

$$
R''=C/R^2,\quad C=qQ/(16\pi\epsilon_0 m),\quad R(0)=R_i,\quad R'(0)=0
$$

$$
t=\sqrt{\frac{R_i^3}{2C}}\left[\sqrt{s(s-1)}+\operatorname{asinh}\sqrt{s-1}\right],\quad s=R/R_i
$$

| Symbols | Units | Meaning |
| --- | --- | --- |
| `R,R_i` | m | half-separation and its initial value |
| `q,Q,m` | C, C, kg | test charge, other represented source charge and particle mass |
| `epsilon_0,C` | F/m, m^3/s^2 | vacuum permittivity and acceleration coefficient |
| `s,t` | 1, s | normalized half-separation and elapsed time |

**Validity:** Equal particle masses, equal positive represented source charges, symmetric rest launch, unsoftened vacuum Coulomb interaction, no external fields. The source charge Q may represent a macro-particle; q/m remains the physical particle charge-to-mass ratio.

**Provenance:** derived here.

1. At separation 2R the force on either particle is qQ/[4 pi epsilon0 (2R)^2], hence R''=C/R^2.
2. Multiply by R' and integrate: R'^2=2C(1/Ri-1/R).
3. Separate variables with s=R/Ri and integrate sqrt(s/(s-1)); the primitive is sqrt(s(s-1))+asinh(sqrt(s-1)).
4. The existing test bisects this monotone implicit time relation and compares the coupled ensemble trajectory.

**Executable checks:**

- [`space_charge.rs::direct_second_order_against_exact_two_body_expansion`](../../crates/simrep-fly/tests/space_charge.rs): `cargo test --release -p simrep-fly --test space_charge direct_second_order_against_exact_two_body_expansion -- --exact --nocapture`

**Band and uncertainty:** Reuse dt=0.04/0.02/0.01 us at t=0.7 us with Ri=1 mm and total represented charge 2e-14 C; adjacent error ratios >3.7, symmetry exact; three-step GCI on final half-separation.

## child-langmuir

Planar cold space-charge-limited diode solution

$$
J_{CL}=\frac49\epsilon_0\sqrt{\frac{2e}{m}}\frac{V_A^{3/2}}{d^2}
$$

$$
V(x)=V_A(x/d)^{4/3},\quad v(x)=\sqrt{2eV(x)/m},\quad\rho(x)=-J_{CL}/v(x)
$$

| Symbols | Units | Meaning |
| --- | --- | --- |
| `J_CL` | A/m^2 | magnitude of electron current density |
| `epsilon_0,e,m` | F/m, C, kg | vacuum permittivity, positive elementary charge and electron mass |
| `V_A,V` | V | anode voltage and potential relative to cathode |
| `x,d,v,rho` | m, m, m/s, C/m^3 | distance from cathode, full gap, electron speed and negative charge density |

**Validity:** One-dimensional infinite plates, steady mono-species collisionless nonrelativistic electrons, zero cathode injection speed and field, no magnetic field. Density is singular at x=0. The SimRep test supplies this analytical density on x/d in [1/8,1] and verifies Poisson/current consistency; it does not claim that a self-consistent emission boundary or limiting-current search is implemented.

**Provenance:** derived here.

1. Energy conservation gives m v^2/2=e V and continuity gives rho=-J/v.
2. Poisson becomes V''=(J/epsilon0) sqrt(m/(2e)) V^(-1/2).
3. Multiply by V' and integrate with V(0)=V'(0)=0: V'^2=4(J/epsilon0)sqrt(m/(2e))sqrt(V).
4. Integrate again and impose V(d)=VA to obtain V=VA(x/d)^(4/3) and the 4/9 current prefactor.
5. Test uses d=1 mm and VA=50 V, converts speed from mm/us to mm/s, and reconstructs J=epsilon0 V'' sqrt(2eV/m) from the solved profile at x/d=9/16.

**Executable checks:**

- [`exact_solutions.rs::child_langmuir_poisson_profile_and_current`](../../crates/simrep-fly/tests/exact_solutions.rs): `cargo test --release -p simrep-fly --test exact_solutions child_langmuir_poisson_profile_and_current -- --exact --nocapture`

**Band and uncertainty:** Relative reconstructed-current tolerance 2e-4; 17/33/65-point axial grids and three-grid GCI. SI current density is converted to A/mm^2 for gate output.

## Deliberate omissions

- `einzel-brillouin-thin-lens`: No universal closed focal-length formula exists without an axial potential profile and an approximation regime. A field-profile integral or numerical ray matrix is not an exact solution specified by electrode voltage and dimensions alone; omitted rather than inventing a closed form.
