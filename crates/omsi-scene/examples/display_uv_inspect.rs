fn main() {
    let m = omsi_o3d::load_mesh(std::path::Path::new(&std::env::args().nth(1).unwrap())).unwrap();
    for (slot, mat) in m.materials.iter().enumerate() {
        let ids: std::collections::BTreeSet<_> = m
            .triangles
            .iter()
            .filter(|t| t.material as usize == slot)
            .flat_map(|t| t.indices)
            .collect();
        println!("material {slot} {}", mat.texture);
        for i in ids {
            let v = m.vertices[i as usize];
            println!("{i} pos {:?} uv {:?}", v.position, v.uv);
        }
    }
}
