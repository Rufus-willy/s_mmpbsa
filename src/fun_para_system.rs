use std::path::Path;
use std::fs;

use crate::parse_pdb::{PDBModel, PDB};
use crate::settings::Settings;
use crate::utils::{append_new_name, get_input_selection, make_ndx, trajectory};
use crate::fun_para_mmpbsa::set_para_mmpbsa;
use crate::index_parser::{Index, IndexGroup};
use crate::parse_tpr::TPR;
use crate::atom_property::AtomProperties;
use crate::parse_tpr::Residue;
use crate::utils::{convert_tpr, convert_trj, trjconv};
use crate::parse_xvg::read_coord_xvg;

pub fn set_para_trj(trj: &String, tpr: &mut TPR, ndx_name: &String, wd: &Path, tpr_name: &str, settings: &mut Settings) {
    let mut receptor_grp: Option<usize> = None;
    let mut ligand_grp: Option<usize> = None;
    let mut bt: f64 = 0.0;                                  // ps
    let mut et: f64 = tpr.dt * tpr.nsteps as f64;           // ps
    let mut dt = 1000.0;                               // ps
    let unit_dt: f64 = tpr.dt * tpr.nstxout as f64;         // ps
    let ndx = Index::from(ndx_name);
    loop {
        println!("\n                 ************ Trajectory Parameters ************");
        println!("-10 Return");
        println!(" -1 Toggle whether to fix PBC conditions, current: {}", settings.fix_pbc);
        println!("  0 Go to next step");
        println!("  1 Select receptor groups, current:          {}", show_grp(receptor_grp, &ndx));
        println!("  2 Select ligand groups, current:            {}", show_grp(ligand_grp, &ndx));
        println!("  3 Set start time to analyze, current:       {} ns", bt / 1000.0);
        println!("  4 Set end time to analyze, current:         {} ns", et / 1000.0);
        println!("  5 Set time interval to analyze, current:    {} ns", dt / 1000.0);
        let i = get_input_selection();
        match i {
            -10 => return,
            -1 => {
                settings.fix_pbc = !settings.fix_pbc;
            }
            0 => {
                if let Some(receptor_grp) = receptor_grp {
                    prepare_system_tpr(receptor_grp, ligand_grp, trj, tpr, &ndx, tpr_name, ndx_name, bt, et, dt, wd, settings);
                } else {
                    println!("Please select receptor groups.");
                };
            }
            1 => {
                println!("Current groups:");
                ndx.list_groups();
                println!("Input receptor group num:");
                receptor_grp = Some(get_input_selection());
            }
            2 => {
                println!("Current groups:");
                ndx.list_groups();
                println!("Input ligand group num (-1 for nothing):");
                ligand_grp = match get_input_selection() {
                    -1 => None,
                    i => Some(i as usize)
                };
            }
            3 => {
                println!("Input start time (ns), should be divisible of {} ps:", dt);
                let mut new_bt = get_input_selection::<f64>() * 1000.0;
                while new_bt * 1000.0 % dt != 0.0 || new_bt > tpr.nsteps as f64 * tpr.dt as f64 || new_bt < 0.0 {
                    println!("The input {} ns not a valid time in trajectory.", new_bt / 1000.0);
                    println!("Input start time (ns) again, should be divisible of {} fs:", dt);
                    new_bt = get_input_selection::<f64>() * 1000.0;
                }
                bt = new_bt;
            }
            4 => {
                println!("Input end time (ns), should be divisible of {} ps:", dt);
                let mut new_et = get_input_selection::<f64>() * 1000.0;
                while new_et * 1000.0 % dt != 0.0 || new_et > tpr.nsteps as f64 * tpr.dt as f64 || new_et < 0.0 {
                    println!("The input {} ns not a valid time in trajectory.", new_et / 1000.0);
                    println!("Input end time (ns) again, should be divisible of {} fs:", dt);
                    new_et = get_input_selection::<f64>() * 1000.0;
                }
                et = new_et;
            }
            5 => {
                println!("Input interval time (ns), should be divisible of {} ps:", unit_dt);
                let mut new_dt = get_input_selection::<f64>() * 1000.0;
                while new_dt * 1000.0 % unit_dt != 0.0 {
                    println!("The input {} ns is not a valid time step.", new_dt / 1000.0);
                    println!("Input interval time (ns) again, should be divisible of {} ps:", unit_dt);
                    new_dt = get_input_selection::<f64>() * 1000.0;
                }
                dt = new_dt;
            }
            _ => println!("Invalid input")
        }
    }
}

fn prepare_pymol_complex_pdb(complex_path: &String) -> PDB {
    let complex_pdb = fs::read_to_string(complex_path).unwrap();
    let complex_pdb: Vec<&str> = complex_pdb.split("\n").collect();
    let model_start_ln: Vec<usize> = complex_pdb.iter().enumerate().filter_map(|(i, &line)| match line.starts_with("MODEL") {
        true => Some(i),
        false => None
    }).collect();
    let first_model_ter: Vec<usize> = complex_pdb.iter().enumerate().filter_map(|(i, &line)| match line.starts_with("TER") {
        true => Some(i),
        false => None
    }).collect();
    let first_model_ter = first_model_ter[0];
    let first_model = PDBModel::from(complex_pdb[model_start_ln[0]..model_start_ln[1]].join("\n").as_str());
    let receptor = PDBModel::from(complex_pdb[model_start_ln[0]..first_model_ter].join("\n").as_str());
    let mut pdb: Vec<PDBModel> = vec![first_model];
    PDB::new(&pdb).to_pdb("MMPBSA_model_0.pdb");
    for i in 1..model_start_ln.len() {
        let ligand_model = if i != model_start_ln.len() - 1 {
            PDBModel::from(complex_pdb[model_start_ln[i]..model_start_ln[i + 1]].join("\n").as_str())
        } else {
            PDBModel::from(complex_pdb[model_start_ln[i]..].join("\n").as_str())
        };
        let mut rec = receptor.clone();
        rec.push_atoms(&ligand_model.atoms);
        pdb.push(rec);
    }
    PDB::new(&pdb)
}

pub fn set_para_trj_pdbqt(complex_path: &String, wd: &Path, settings: &mut Settings) {
    let pdb = prepare_pymol_complex_pdb(complex_path);
    let ndx = Index::from(&wd.join("MMPBSA_index.ndx").to_str().unwrap().to_string());

    let mut receptor_grp: Option<usize> = None;
    let mut ligand_grp: Option<usize> = None;
    let mut bt: usize = 0;
    let mut et: usize = pdb.models.len() - 1;
    loop {
        println!("\n                 ************ Trajectory Parameters ************");
        println!("-10 Return");
        println!("  0 Go to next step");
        println!("  1 Select receptor groups, current:          {}", show_grp(receptor_grp, &ndx));
        println!("  2 Select ligand groups, current:            {}", show_grp(ligand_grp, &ndx));
        println!("  3 Set start pose to analyze, current:       {}", bt + 1);
        println!("  4 Set end pose to analyze, current:         {}", et + 1);
        let i = get_input_selection();
        match i {
            -10 => return,
            -1 => {
                settings.fix_pbc = !settings.fix_pbc;
            }
            0 => {
                if let Some(receptor_grp) = receptor_grp {
                    prepare_system_tpr_pdb(receptor_grp, ligand_grp, &pdb, &ndx, bt, et, wd, settings);
                } else {
                    println!("Please select receptor groups.");
                };
            }
            1 => {
                println!("Current groups:");
                ndx.list_groups();
                println!("Input receptor group num:");
                receptor_grp = Some(get_input_selection());
            }
            2 => {
                println!("Current groups:");
                ndx.list_groups();
                println!("Input ligand group num (-1 for nothing):");
                ligand_grp = match get_input_selection::<i32>() {
                    -1 => None,
                    i => Some(i as usize)
                };
            }
            3 => {
                println!("Input start pose, should be integer:");
                let mut new_bt = get_input_selection::<usize>() - 1;
                while new_bt > pdb.models.len() {
                    println!("The input {} not a valid pose in trajectory.", new_bt);
                    println!("Input start pose again:");
                    new_bt = get_input_selection::<usize>() - 1;
                }
                bt = new_bt;
            }
            4 => {
                println!("Input end pose, should be integer:");
                let mut new_et = get_input_selection::<usize>() - 1;
                while new_et > pdb.models.len() {
                    println!("The input {} not a valid pose in trajectory.", new_et);
                    println!("Input end pose again:");
                    new_et = get_input_selection::<usize>() - 1;
                }
                et = new_et;
            }
            _ => println!("Invalid input")
        }
    }
}

// convert rec and lig to begin at 0 and continous
pub fn normalize_index(ndx_rec: &Vec<usize>, ndx_lig: Option<&Vec<usize>>) -> (Vec<usize>, Vec<usize>) {
    if let Some(ndx_lig) = ndx_lig {
        let mut ndx_rec_norm = ndx_rec.clone();
        let mut ndx_lig_norm = ndx_lig.clone();
        let last_atom = ndx_rec.len() + ndx_lig.len() - 1;
        for cur_atom_id in 0..=last_atom {
            if !ndx_lig_norm.contains(&cur_atom_id) && !ndx_rec_norm.contains(&cur_atom_id) {
                let ndx_lig_norm2 = ndx_lig_norm.clone();
                let ndx_rec_norm2 = ndx_rec_norm.clone();
                let next_edge_lig = ndx_lig_norm2.iter().find(|&&i| i > cur_atom_id);
                let next_edge_rec = ndx_rec_norm2.iter().find(|&&i| i > cur_atom_id);
                let offset = if next_edge_lig.is_none() {
                    if next_edge_rec.is_none() {
                        0
                    } else {
                        next_edge_rec.unwrap() - cur_atom_id
                    }
                } else {
                    if next_edge_rec.is_none() {
                        next_edge_lig.unwrap() - cur_atom_id
                    } else {
                        next_edge_lig.unwrap().min(next_edge_rec.unwrap()) - cur_atom_id
                    }
                };
                ndx_lig_norm.iter_mut().for_each(|i| if *i > cur_atom_id { *i -= offset } );
                ndx_rec_norm.iter_mut().for_each(|i| if *i > cur_atom_id { *i -= offset } );
            }
        }
        (ndx_rec_norm, ndx_lig_norm)
    } else {
        ((0..ndx_rec.len()).collect(), (0..ndx_rec.len()).collect())
    }
}

pub fn get_residues_tpr(tpr: &TPR, ndx_com: &Vec<usize>) -> Vec<Residue> {
    let mut residues: Vec<Residue> = vec![];
    let mut idx = 0;
    let mut resind_offset = 0;
    
    for mol in &tpr.molecules {
        for _ in 0..tpr.molecule_types[mol.molecule_type_id].molecules_num {
            for atom in &mol.atoms {
                idx += 1;
                if ndx_com.contains(&idx) && residues.len() <= atom.resind + resind_offset {
                    residues.push(mol.residues[atom.resind].to_owned());
                }
            }
            resind_offset += mol.residues.len();
        }
    }
    residues
}

fn show_grp(grp_id: Option<usize>, ndx: &Index) -> String {
    if let Some(grp_id) = grp_id {
        if let Some(grp) = ndx.groups.get(grp_id) {
            format!("{}): {}", grp_id, grp)
        } else {
            String::from("undefined")
        }
    } else {
        String::from("undefined")
    }
}

fn prepare_system_tpr(receptor_grp: usize, ligand_grp: Option<usize>, 
                  trj: &String, tpr: &mut TPR, ndx: &Index, 
                  tpr_name: &str, ndx_name: &String, 
                  bt: f64, et: f64, dt: f64, 
                  wd: &Path, settings: &mut Settings) {
    // atom indexes
    println!("Preparing atom indexes...");
    let ndx_lig = match ligand_grp {
        Some(ligand_grp) => Some(&ndx.groups[ligand_grp].indexes),
        None => None
    };
    let ndx_rec = &ndx.groups[receptor_grp].indexes;
    let ndx_com = match ndx_lig {
        Some(ndx_lig) => {
            match ndx_lig[0] > ndx_rec[0] {
                true => {
                    let mut ndx_com = ndx_rec.to_vec();
                    ndx_com.extend(ndx_lig);
                    ndx_com
                }
                false => {
                    let mut ndx_com = ndx_lig.to_vec();
                    ndx_com.extend(ndx_rec);
                    ndx_com
                }
            }
        }
        None => ndx_rec.to_vec()
    };

    // atom properties
    println!("Parsing atom properties...");
    let mut aps = AtomProperties::from_tpr(tpr, &ndx_com);
    println!("Collecting residues list...");
    let residues = get_residues_tpr(tpr, &ndx_com);

    // pre-treat trajectory: fix pbc
    let trj_mmpbsa = append_new_name(trj, ".xtc", "_MMPBSA_"); // get trj output file name
    let tpr_name = append_new_name(tpr_name, ".tpr", ""); // fuck the passed tpr name is dump
    
    // step 1: generate new index
    println!("Generating Index...");
    // gmx make_ndx -f md.tpr -n index.idx -o md_trj_whole.xtc -pbc whole
    let ndx_whole = append_new_name(ndx_name, "_whole.ndx", "_MMPBSA_"); // get extracted index file name
    if let Some(ligand_grp) = ligand_grp {
        make_ndx(&vec![
            format!("{} | {}", receptor_grp, ligand_grp).as_str(),
            format!("name {} Complex", ndx.groups.len()).as_str(),
            format!("name {} Receptor", receptor_grp).as_str(),
            format!("name {} Ligand", ligand_grp).as_str(),
            "q"
        ], wd, settings, &tpr_name, ndx_name, &ndx_whole);
    } else {
        make_ndx(&vec![
            // complex is receptor
            format!("name {} Complex", receptor_grp).as_str(),
            "q"
        ], wd, settings, &tpr_name, ndx_name, &ndx_whole);
    }
    
    // step 2: extract new trj with old tpr and new index
    println!("Extracting trajectory, be patient...");
    let (bt, et, dt) = (bt.to_string(), et.to_string(), dt.to_string());
    let other_params = vec![
        "-b", &bt,
        "-e", &et,
        "-dt", &dt
    ];
    trjconv(&vec!["Complex"], wd, settings, trj, &tpr_name, &ndx_whole, &trj_mmpbsa, &other_params);
    
    // step 3: extract new tpr from old tpr
    let tpr_mmpbsa = append_new_name(&tpr_name, ".tpr", "_MMPBSA_"); // get extracted tpr file name
    convert_tpr(&vec!["Complex"], wd, settings, &tpr_name, &ndx_whole, &tpr_mmpbsa);
    if !settings.debug_mode {
        fs::remove_file(&ndx_whole).unwrap();
    }
    
    // step 4: generate new index with new tpr
    println!("Normalizing index...");
    let (ndx_rec, ndx_lig) = 
        normalize_index(&ndx.groups[receptor_grp].indexes, match ligand_grp {
            Some(ligand_grp) => Some(&ndx.groups[ligand_grp].indexes),
            None => None
        });
    // 需要处理一下atom_properties的id
    aps.atom_props.iter_mut().enumerate().for_each(|(i, ap)| ap.id = i);
    
    // extract index file
    let ndx_mmpbsa = match ligand_grp {
        Some(_) => {
            Index::new(vec![
                IndexGroup::new("Complex", &ndx_rec.iter().chain(ndx_lig.iter()).cloned().collect()), 
                IndexGroup::new("Receptor", &ndx_rec),
                IndexGroup::new("Ligand", &ndx_lig)
            ])
        },
        None => {
            Index::new(vec![
                IndexGroup::new("Complex", &ndx_rec)
            ])
        }
    };
    ndx_mmpbsa.to_ndx(wd.join("_MMPBSA_index.ndx").to_str().unwrap());
    let ndx_mmpbsa = wd.join("_MMPBSA_index.ndx");
    let ndx_mmpbsa = ndx_mmpbsa.to_str().unwrap();
    // 在这里 remove pbc, convert-trj有bug, 不能处理不完整蛋白, 故先trjconv再convert-trj
    if settings.fix_pbc {
        let other_params = vec!["-rmpbc", "-select", "Complex"];
        convert_trj(&vec![], wd, settings, &trj_mmpbsa, &tpr_name, &ndx_mmpbsa, &trj_mmpbsa, &other_params);
    }

    println!("Loading trajectory coordinates...");
    trajectory(&vec!["Complex"], wd, settings, &trj_mmpbsa, &tpr_mmpbsa, &ndx_mmpbsa, "_MMPBSA_coord.xvg");
    let (time_list, coordinates) = read_coord_xvg(wd.join("_MMPBSA_coord.xvg").to_str().unwrap());

    set_para_mmpbsa(&time_list, &coordinates, tpr, &ndx, wd, &mut aps, &ndx_rec, &ndx_lig, receptor_grp, ligand_grp, &residues, settings);
}

fn prepare_system_tpr_pdb(receptor_grp: usize, ligand_grp: Option<usize>, 
                          pdb: &PDB, ndx: &Index, 
                          bt: usize, et: usize, wd: &Path, settings: &Settings) {
    println!("不干了");
}