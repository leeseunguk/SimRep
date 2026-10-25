# PA electrode geometry during flight

The default staircase-PA collision body consists of fully occupied grid cells.
All four corners must be electrode nodes in planar or cylindrical 2D, and all
eight in 3D. An invariant single-point axis contributes one corner. Mirror
coordinates are folded before locating a cell; cylindrical PAs locate it in
axial x and radial y, then intersect the corresponding cylindrical shells.

`GridField::status` and segment intersections use the same rule. Segment
partitioning occurs at integer node coordinates, not halfway between nodes.
This detects a thin occupied cell even when both accepted endpoints and all
integration stages lie in vacuum. Transparent PA faces retain their Dirichlet
field values but do not contribute absorbing corners.

For the app quadrupole at 0.5 mm spacing, the node-cell rule places the
downstream plate entrance at x=53 mm. A zero-voltage SIMION probe passes the
aperture at radius 4.49 mm and splats at radius 4.5 mm. Its rod splat is
|z|=5.5 mm for y=0..1 mm and |z|=6 mm at y=2 mm. These are staircase surfaces,
not the template's physical round rods or its radius-4 mm aperture.
`tools/simion-oracle/quad_collision_probe.py` reproduces the measurements
without electric deflection in a separate copied workbench.

The earlier nearest-node convention remains explicit:

```rust
let field = simrep_fly::GridField::new(array)?
    .with_nearest_node_collisions();
# Ok::<(), simrep_fly::FlyError>(())
```

The CLI spelling is `simrep fly --nearest-node-collisions`. It uses half-grid
voxels clipped to the PA extent and applies the same choice to all voltage
bases. It is a comparison option, not a reconstruction or solver option.
For example, it moves the quadrupole's plate entrance to x=52.75 mm and its
innermost rod voxel to radius 4.75 mm. An isolated electrode node has an
absorbing voxel under this convention but no occupied cell under the default.

A physical surface model defines a third problem. The quadrupole GEM in
`bench/quadrupole.gem` has radius-5.75 mm round rods at transverse offset
10.75 mm and radius-4 mm circular apertures. A cut-cell potential solve and
surface collision model must use that same physical geometry. Refining a PA
mask while changing collision voxels does not establish convergence of any
one of these problems. `bench/fly-real.md` reports them separately.
