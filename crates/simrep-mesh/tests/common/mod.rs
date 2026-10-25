use simrep_mesh::Part;

pub fn cube(low: f64, high: f64) -> Part {
    Part {
        name: "cube".into(),
        vertices: vec![
            [low, low, low],
            [high, low, low],
            [high, high, low],
            [low, high, low],
            [low, low, high],
            [high, low, high],
            [high, high, high],
            [low, high, high],
        ],
        triangles: vec![
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ],
    }
}
