use std::collections::HashMap;
use std::env::current_exe;
use std::fs;
use std::path::Path;
use crate::atom_property::AtomProperties;
use crate::parse_tpr::TPR;

impl AtomProperties {
    // ff_radius would not be used
    pub fn apply_radius(&mut self, radius_type: usize, at_list: &Vec<String>, radius_types: &Vec<&str>, wd: &Path) {
        let rad_type = radius_types[radius_type];
        if rad_type.ne("ff") {
            let radii_table = get_radii_map(rad_type);
            for (i, r) in &mut self.atom_props.iter_mut().enumerate() {
                r.radius = get_radii(&radii_table, &at_list[i]);
            }
            self.radius_type = rad_type.to_string();
        } else {
            let radii = fs::read_to_string(wd.join("ff_radius.dat")).unwrap();
            let radii: Vec<&str> = radii.split("\n").collect();
            for (i, r) in &mut self.atom_props.iter_mut().enumerate() {
                r.radius = radii[i].parse().unwrap();
            }
        }
    }
}

// get atom radius from dat
pub fn get_radii(radii_table: &HashMap<String, f64>, at_type: &str) -> f64 {
    if at_type.len() >= 2 {
        match radii_table.get(&at_type[0..2]) {
            Some(&m) => m,
            _ => {
                match radii_table.get(&at_type[0..1]) {
                    Some(&m) => m,
                    _ => radii_table["*"]
                }
            }
        }
    } else {
        match radii_table.get(at_type) {
            Some(&m) => m,
            _ => radii_table["*"]
        }
    }
}

impl TPR {
    pub fn get_at_list(&self) -> Vec<String> {
        let mut atom_radius: Vec<String> = vec![];
        for mol in &self.molecules {
            for _ in 0..self.molecule_types[mol.molecule_type_id].molecules_num {
                for atom in &mol.atoms {
                    let at = atom.name.to_uppercase();
                    atom_radius.push(at);
                }
            }
        }
        atom_radius
    }
}

pub fn get_radii_map(rad_type: &str) -> HashMap<String, f64> {
    let mut radii_table: HashMap<String, f64> = HashMap::new();
    let radii_file = current_exe().expect("Cannot get current s_mmpbsa program path.")
        .parent().expect("Cannot get current s_mmpbsa program directory.")
        .join("dat").join(format!("{}.dat", &rad_type))
        .to_str().expect("The atom radius data files (dat/) not found.").to_string();
    let radii_file_content = fs::read_to_string(radii_file)
        .expect(format!("Error reading atom radius data file: {}.dat", rad_type).as_str());
    for l in radii_file_content.split("\n").filter(|p| !p.trim().starts_with("//") && !p.trim().is_empty()) {
        let k_v: Vec<&str> = l.split(":").collect();
        radii_table.insert(k_v[0].to_string(), k_v[1].trim().parse::<f64>().unwrap());
    }
    radii_table
}
