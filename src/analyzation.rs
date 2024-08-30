use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::process::exit;
use indicatif::ProgressBar;
use ndarray::{s, Array1, Array2, Array3, Axis};
use serde::{Deserialize, Serialize};
use crate::mmpbsa::set_style;
use crate::parse_tpr::Residue;
use crate::settings::Settings;
use crate::utils::{get_input, get_input_selection, get_residue_range_ca, range2list};

#[derive(Clone, Serialize, Deserialize)]
pub struct SMResult {
    pub mutation: String,
    pub atom_names: Vec<String>,
    pub atom_res: Vec<usize>,
    pub residues: Vec<Residue>,
    pub ndx_lig: Vec<usize>,
    pub times: Vec<f64>,
    pub coord: Array3<f64>,
    pub dh: Array1<f64>,
    pub mm: Array1<f64>,
    pub pb: Array1<f64>,
    pub sa: Array1<f64>,
    pub elec: Array1<f64>,
    pub vdw: Array1<f64>,
    pub dh_atom: Array2<f64>,
    pub mm_atom: Array2<f64>,
    pub pb_atom: Array2<f64>,
    pub sa_atom: Array2<f64>,
    pub elec_atom: Array2<f64>,
    pub vdw_atom: Array2<f64>,
}

impl SMResult {
    pub fn new(atom_names: &Vec<String>, atom_res: &Vec<usize>, 
               residues: &Vec<Residue>, ndx_lig: &Vec<usize>, 
               times: &Vec<f64>, coord: &Array3<f64>, mutation: &str,
               elec_atom: &Array2<f64>, vdw_atom: &Array2<f64>, 
               pb_atom: &Array2<f64>, sa_atom: &Array2<f64>) -> SMResult {
        let mut dh: Array1<f64> = Array1::zeros(times.len());
        let mut mm: Array1<f64> = Array1::zeros(times.len());
        let mut pb: Array1<f64> = Array1::zeros(times.len());
        let mut sa: Array1<f64> = Array1::zeros(times.len());
        let mut elec: Array1<f64> = Array1::zeros(times.len());
        let mut vdw: Array1<f64> = Array1::zeros(times.len());
        for t in 0..times.len() {
            elec[t] = elec_atom.row(t).sum();
            vdw[t] = vdw_atom.row(t).sum();
            mm[t] = elec[t] + vdw[t];
            pb[t] = pb_atom.row(t).sum();
            sa[t] = sa_atom.row(t).sum();
            dh[t] = mm[t] + pb[t] + sa[t];
        }

        let mm_atom: Array2<f64> = elec_atom + vdw_atom;
        let dh_atom: Array2<f64> = &mm_atom + pb_atom + sa_atom;

        SMResult {
            mutation: mutation.to_string(),
            atom_names: atom_names.to_vec(),
            atom_res: atom_res.to_vec(),
            residues: residues.to_owned(),
            ndx_lig: ndx_lig.to_owned(),
            times: times.to_owned(),
            coord: coord.to_owned(),
            dh,
            mm,
            pb,
            sa,
            elec,
            vdw,
            dh_atom,
            mm_atom,
            pb_atom: pb_atom.to_owned(),
            sa_atom: sa_atom.to_owned(),
            elec_atom: elec_atom.to_owned(),
            vdw_atom: vdw_atom.to_owned(),
        }
    }

    pub fn to_bin(&self, target: &Path) {
        println!("Saving results to {}", target.to_str().unwrap());
        let mut result_as_serialize = std::fs::File::create(target).unwrap();
        serde_pickle::to_writer(&mut result_as_serialize, self, serde_pickle::SerOptions::new()).unwrap();
    }

    pub fn from(result_serialize: &str) -> SMResult {
        let result_deserialize = std::fs::File::open(result_serialize).unwrap();
        serde_pickle::from_reader(&result_deserialize, serde_pickle::DeOptions::new()).unwrap()
    }
}

pub fn analyze_controller(result_wt: &SMResult, result_as: &Vec<SMResult>, temperature: f64, sys_name: &String, wd: &Path, settings: &Settings) {
    let mut results = result_as.clone();
    results.insert(0, result_wt.clone());
    loop {
        println!("\n                 ************ MM-PBSA analyzation ************");
        println!("Time range: {} - {} ns, step = {} ns\n", result_wt.times[0], result_wt.times.last().unwrap(), if result_wt.times.len() > 1 {
            result_wt.times[1] - result_wt.times[0]
        } else {
            0.0
        });
        println!("-1 Write residue-wised binding energy at specific time to pdb file");
        println!(" 0 Exit program");
        println!(" 1 View binding energy summary");
        println!(" 2 Output binding energy by trajectory");
        println!(" 3 Output binding energy by residue at specific time");
        println!(" 4 Output ligand binding energy by atom at specific time");
        println!("10 Output residue-wised binding energy by time as default names");
        let sel_fun: i32 = get_input_selection();
        match sel_fun {
            -1 => {
                let ts_ids = get_time_points(result_wt);
                let pb = ProgressBar::new((results.len() * ts_ids.len()) as u64);
                set_style(&pb);
                for result in &results {
                    for ts_id in &ts_ids {
                        let def_name = format!("MMPBSA_binding_energy_{}_{}ns.pdb", sys_name, result.times[*ts_id]);
                        write_pdb_with_bf(result, &def_name, *ts_id, wd, &(0..result.atom_names.len()).collect(), true);
                        pb.inc(1);
                    }
                }
                pb.finish();
                println!("Finished writing pdb file(s) with binding energy information.");
            },
            0 => exit(0),
            1 => {
                for result in &results {
                    analyze_summary(result, temperature, wd, &format!("{}-{}", sys_name, result.mutation), settings)
                }
            },
            2 => {
                for result in &results {
                    analyze_traj(result, wd, &format!("{}-{}", sys_name, result.mutation))
                }
            },
            3 => {
                println!("Input the time point (in ns) to output (default: average):");
                let ts = get_input(-1.0);
                for result in &results {
                    analyze_res(result, wd, &format!("{}-{}", sys_name, result.mutation), ts)
                }
            },
            4 => {
                for result in &results {
                    analyze_atom(result, wd, &format!("{}-{}", sys_name, result.mutation))
                }
                println!("Finished writing atom-wised binding energy pdb file(s) for ligand.");
            },
            10 => {
                for result in &results {
                    output_all_details(result, wd, &format!("{}-{}", sys_name, result.mutation))
                }
            },
            _ => println!("Invalid input")
        }
    }
}

fn get_time_points(result: &SMResult) -> Vec<usize> {
    println!("Input the time point (in ns) to write pdb (default: all):");
    let ts = get_input(-1.0);
    println!("Writing pdb file(s)...");
    if ts != -1.0 {
        if let Some(ts_id) = get_time_index(ts, result) {
            vec![ts_id]
        } else {
            println!("Error input: {} ns", ts);
            vec![]
        }
    } else {
        Vec::from_iter(0..result.times.len())
    }
}

fn get_time_index(ts: f64, results: &SMResult) -> Option<usize> {
    if ts == results.times[0] {
        Some(0)
    } else {
        if results.times.len() > 1 {
            Some(((ts - results.times[0]) / (results.times[1] - results.times[0])) as usize)
        } else {
            None
        }
    }
}

fn write_pdb_with_bf(result: &SMResult, def_name: &String, ts_id: usize, wd: &Path, atom_range: &Vec<usize>, by_frame: bool) {
    let mut f = fs::File::create(wd.join(def_name)).unwrap();
    let coord = &result.coord;
    writeln!(f, "REMARK  Generated by s_mmpbsa (https://github.com/supernova4869/s_mmpbsa)").unwrap();
    writeln!(f, "REMARK  B-factor column filled with INVERSED receptor-ligand interaction energy (kJ/mol)").unwrap();
    for (id, &res_id) in result.atom_res.iter().enumerate() {
        if atom_range.contains(&id) {
            write_atom_line(&result, id, &result.atom_names[id], res_id, ts_id, 
                coord[[ts_id, id, 0]], coord[[ts_id, id, 1]], coord[[ts_id, id, 2]], &mut f, by_frame);
        }
    }
    writeln!(f, "END").unwrap();
}

fn write_atom_line(result: &SMResult, id: usize, name: &String, res_id: usize, ts_id: usize, x: f64, y: f64, z: f64, f: &mut File, by_frame: bool) {
    if by_frame {
        writeln!(f, "ATOM  {:5} {:<4} {:<3} A{:4}    {:8.3}{:8.3}{:8.3}  1.00{:6.2}           {:<2}", 
                    id + 1, name, result.residues[res_id].name, result.residues[res_id].nr, x, y, z, 
                    -result.dh_atom[[ts_id, id]], name.get(0..1).unwrap()).unwrap();
    } else {
        let dh_avg = result.dh_atom.mean_axis(Axis(0)).unwrap();
        writeln!(f, "ATOM  {:5} {:<4} {:<3} A{:4}    {:8.3}{:8.3}{:8.3}  1.00{:6.2}           {:<2}", 
                    id + 1, name, result.residues[res_id].name, result.residues[res_id].nr, x, y, z, 
                    -dh_avg[id], name.get(0..1).unwrap()).unwrap();
    }
}

fn analyze_summary(results: &SMResult, temperature: f64, wd: &Path, sys_name: &String, settings: &Settings) {
    let rt2kj = 8.314462618 * temperature / 1e3;

    let dh_avg = results.dh.mean().unwrap();
    let mm_avg = results.mm.mean().unwrap();
    let elec_avg = results.elec.mean().unwrap();
    let vdw_avg = results.vdw.mean().unwrap();
    let pb_avg = results.pb.mean().unwrap();
    let sa_avg = results.sa.mean().unwrap();

    let tds = match settings.use_ts {
        true => {
            -rt2kj * (results.mm.iter().map(|&p| f64::exp((p - mm_avg) / rt2kj)).sum::<f64>() / results.mm.len() as f64).ln()
        }
        false => 0.0
    };
    let dg = dh_avg - tds;
    let ki = if dg < 0.0 {
        f64::exp(dg / rt2kj) * 1e9    // nM
    } else {
        0.0
    };

    println!("\nEnergy terms summary:");
    println!("ΔH: {:.3} kJ/mol", dh_avg);
    println!("ΔMM: {:.3} kJ/mol", mm_avg);
    println!("ΔPB: {:.3} kJ/mol", pb_avg);
    println!("ΔSA: {:.3} kJ/mol", sa_avg);
    println!();
    println!("Δelec: {:.3} kJ/mol", elec_avg);
    println!("Δvdw: {:.3} kJ/mol", vdw_avg);
    println!();
    println!("TΔS: {:.3} kJ/mol", tds);
    println!("ΔG: {:.3} kJ/mol", dg);
    if ki != 0.0 {
        println!("Ki: {:.3} nM", ki);
    } else {
        println!("Ki: Unavailable");
    }

    let def_name = format!("MMPBSA_{}.csv", sys_name);
    println!("Writing binding energy terms...");
    let mut energy_sum = fs::File::create(wd.join(&def_name)).unwrap();
    write!(energy_sum, "Energy Term,value,info\n").unwrap();
    write!(energy_sum, "ΔH,{:.3},ΔH=ΔMM+ΔPB+ΔSA (kJ/mol)\n", dh_avg).unwrap();
    write!(energy_sum, "ΔMM,{:.3},ΔMM=Δelec+ΔvdW (kJ/mol)\n", mm_avg).unwrap();
    write!(energy_sum, "ΔPB,{:.3},(kJ/mol)\n", pb_avg).unwrap();
    write!(energy_sum, "ΔSA,{:.3},(kJ/mol)\n", sa_avg).unwrap();
    write!(energy_sum, "\n").unwrap();
    write!(energy_sum, "Δelec,{:.3},(kJ/mol)\n", elec_avg).unwrap();
    write!(energy_sum, "ΔvdW,{:.3},(kJ/mol)\n", vdw_avg).unwrap();
    write!(energy_sum, "\n").unwrap();
    write!(energy_sum, "TΔS,{:.3},(kJ/mol)\n", tds).unwrap();
    write!(energy_sum, "ΔG,{:.3},ΔG=ΔH-TΔS (kJ/mol)\n", dg).unwrap();
    if ki != 0.0 {
        write!(energy_sum, "Ki,{:.3e},Ki=exp(ΔG/RT) (nM)\n", ki).unwrap();
    } else {
        write!(energy_sum, "Ki,Unavailable,Ki=exp(ΔG/RT) (nM)\n").unwrap();
    }
    println!("Binding energy terms have been writen to {}", &def_name);
}

fn analyze_traj(results: &SMResult, wd: &Path, sys_name: &String) {
    println!("Writing binding energy terms...");
    let def_name = format!("MMPBSA_{}_traj.csv", sys_name);
    let mut energy_sum = fs::File::create(wd.join(&def_name)).unwrap();
    write!(energy_sum, "Time (ns),ΔH,ΔMM,ΔPB,ΔSA,Δelec,ΔvdW\n").unwrap();
    for i in 0..results.times.len() {
        write!(energy_sum, "{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}\n",
                            results.times[i], results.dh[i],
                            results.mm[i], results.pb[i], results.sa[i],
                            results.elec[i], results.vdw[i]).unwrap();
    }
    println!("Binding energy terms have been writen to {}", &def_name);
}

fn analyze_res(results: &SMResult, wd: &Path, sys_name: &String, ts: f64) {
    println!("Determine the residue range to output:");
    println!(" 1 Ligand and receptor residues by: CA within 4 A");
    println!(" 2 Ligand and receptor residues by: CA within 6 A");
    println!(" 3 Ligand and receptor residues by: CA within 8 A");
    println!(" 4 Ligand and receptor residues by: CA within a specified distance");
    println!(" 5 Self-defined residue range");
    // 残基范围确定
    let i: i32 = get_input_selection();
    let mut range_des = String::from("4A");
    let target_res = match i {
        1 => {
            get_residue_range_from_results(results, 4.0)
        },
        2 => {
            range_des = String::from("6A");
            get_residue_range_from_results(results, 6.0)
        },
        3 => {
            range_des = String::from("8A");
            get_residue_range_from_results(results, 8.0)
        },
        4 => {
            println!("Input the cut-off distance you want to expand from ligand, default: 4");
            let cutoff = get_input(4.0);
            range_des = format!("{:.1}A", cutoff);
            get_residue_range_from_results(results, cutoff)
        },
        5 => {
            println!("Input the residue range you want to output (e.g., 1-3, 5), default: all");
            let res_range = get_input(String::new());
            range_des = res_range.to_string();
            let res_range: Vec<i32> = match res_range.len() {
                0 => {
                    range_des = "all".to_string();
                    results.residues.iter().map(|r| r.nr).collect()
                },
                _ => range2list(&res_range)
            };
            results.atom_res
                .iter()
                .filter(|&&i| res_range.contains(&(results.residues[i].nr)))    // 用户筛选用nr
                .map(|&i| results.residues[i].id)     // 索引用id
                .collect()
        },
        _ => {
            println!("Invalid selection");
            return
        }
    };
    
    println!("Writing energy file(s)...");
    if ts != -1.0 {
        if let Some(ts_id) = get_time_index(ts, results) {
            let def_name = format!("MMPBSA_{}_res_{}_{}ns.csv", sys_name, range_des, results.times[ts_id]);
            write_res_csv(results, ts_id, wd, &target_res, &def_name);
        } else {
            println!("Error input: {} ns", ts);
            return;
        }
    } else {
        let def_name = format!("MMPBSA_{}_res_{}.csv", sys_name, range_des);
        write_res_avg_csv(results, wd, &target_res, &def_name);
    }

    println!("Finished writing residue-wised binding energy file(s).");
}

fn analyze_atom(results: &SMResult, wd: &Path, sys_name: &String) {
    let def_name = format!("MMPBSA_{}_ligand.pdb", sys_name);
    write_pdb_with_bf(results, &def_name, 0, wd, &results.ndx_lig, false);
}

fn write_res_csv(results: &SMResult, ts_id: usize, wd: &Path, target_res: &Vec<usize>, def_name: &String) {
    let mut energy_res = fs::File::create(wd.join(def_name)).unwrap();
    energy_res.write_all("id,name,ΔH,ΔMM,ΔPB,ΔSA,Δelec,ΔvdW\n".as_bytes()).unwrap();
    for res in results.residues.iter() {
        if !target_res.contains(&res.id) {
            continue;
        }
        write!(energy_res, "{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}\n", 
            res.nr, res.name,
            results.atom_res.iter().filter_map(|&a| if a == res.id {
                Some(results.dh_atom[[ts_id, a]])
            } else {
                None
            } ).sum::<f64>(),
            results.atom_res.iter().filter_map(|&a| if a == res.id {
                Some(results.mm_atom[[ts_id, a]])
            } else {
                None
            } ).sum::<f64>(),
            results.atom_res.iter().filter_map(|&a| if a == res.id {
                Some(results.pb_atom[[ts_id, a]])
            } else {
                None
            } ).sum::<f64>(),
            results.atom_res.iter().filter_map(|&a| if a == res.id {
                Some(results.sa_atom[[ts_id, a]])
            } else {
                None
            } ).sum::<f64>(),
            results.atom_res.iter().filter_map(|&a| if a == res.id {
                Some(results.elec_atom[[ts_id, a]])
            } else {
                None
            } ).sum::<f64>(),
            results.atom_res.iter().filter_map(|&a| if a == res.id {
                Some(results.vdw_atom[[ts_id, a]])
            } else {
                None
            } ).sum::<f64>())
            .expect("Error while writing residue-wised energy file");
    }
}

fn write_res_avg_csv(results: &SMResult, wd: &Path, target_res: &Vec<usize>, def_name: &String) {
    let mut energy_res = fs::File::create(wd.join(def_name)).unwrap();
    energy_res.write_all("id,name,ΔH,ΔMM,ΔPB,ΔSA,Δelec,ΔvdW\n".as_bytes()).unwrap();
    for res in results.residues.iter() {
        if !target_res.contains(&res.id) {
            continue;
        }
        write!(energy_res, "{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}\n", 
            res.nr, res.name, 
            results.atom_res.iter().filter_map(|&a| if a == res.id {
                Some(results.dh_atom.column(a).sum())
            } else {
                None
            } ).sum::<f64>() / results.times.len() as f64,
            results.atom_res.iter().filter_map(|&a| if a == res.id {
                Some(results.mm_atom.column(a).sum())
            } else {
                None
            } ).sum::<f64>() / results.times.len() as f64,
            results.atom_res.iter().filter_map(|&a| if a == res.id {
                Some(results.pb_atom.column(a).sum())
            } else {
                None
            } ).sum::<f64>() / results.times.len() as f64,
            results.atom_res.iter().filter_map(|&a| if a == res.id {
                Some(results.sa_atom.column(a).sum())
            } else {
                None
            } ).sum::<f64>() / results.times.len() as f64,
            results.atom_res.iter().filter_map(|&a| if a == res.id {
                Some(results.elec_atom.column(a).sum())
            } else {
                None
            } ).sum::<f64>() / results.times.len() as f64,
            results.atom_res.iter().filter_map(|&a| if a == res.id {
                Some(results.vdw_atom.column(a).sum())
            } else {
                None
            } ).sum::<f64>() / results.times.len() as f64)
            .expect("Error while writing residue-wised energy file");
    }
}

fn get_residue_range_from_results(results: &SMResult, cutoff: f64) -> Vec<usize> {
    let total_frames = results.times.len() - 1;
    get_residue_range_ca(&results.coord.slice(s![total_frames, .., ..]).to_owned(), 
        &results.ndx_lig, cutoff, &results.atom_res, &results.atom_names, &results.residues)
}


fn analyze_dh_res_traj(results: &SMResult, wd: &Path, def_name: &String) {
    println!("Writing binding energy terms...");
    let mut energy_res = fs::File::create(wd.join(&def_name)).unwrap();
    energy_res.write_all("Time (ns)".as_bytes()).unwrap();
    for res in &results.residues {
        energy_res.write_all(format!(",{}#{}", res.nr, res.name).as_bytes()).unwrap();
    }
    for i in 0..results.times.len() {
        energy_res.write_all(format!("\n{}", results.times[i]).as_bytes()).unwrap();
        for dh in &results.dh_atom.row(i) {
            energy_res.write_all(format!(",{:.3}", dh).as_bytes()).unwrap();
        }
    }
    energy_res.write_all("\n".as_bytes()).unwrap();
    println!("Binding energy terms have been writen to {}", &def_name);
}

fn analyze_mm_res_traj(results: &SMResult, wd: &Path, def_name: &String) {
    println!("Writing binding energy terms...");
    let mut energy_res = fs::File::create(wd.join(&def_name)).unwrap();
    energy_res.write_all("Time (ns)".as_bytes()).unwrap();
    for res in &results.residues {
        energy_res.write_all(format!(",{}#{}", res.nr, res.name).as_bytes()).unwrap();
    }
    for i in 0..results.times.len() {
        energy_res.write_all(format!("\n{}", results.times[i]).as_bytes()).unwrap();
        for mm in &results.mm_atom.row(i) {
            energy_res.write_all(format!(",{:.3}", mm).as_bytes()).unwrap();
        }
    }
    energy_res.write_all("\n".as_bytes()).unwrap();
    println!("Binding energy terms have been writen to {}", &def_name);
}

fn analyze_pb_res_traj(results: &SMResult, wd: &Path, def_name: &String) {
    println!("Writing binding energy terms...");
    let mut energy_res = fs::File::create(wd.join(&def_name)).unwrap();
    energy_res.write_all("Time (ns)".as_bytes()).unwrap();
    for res in &results.residues {
        energy_res.write_all(format!(",{}#{}", res.nr, res.name).as_bytes()).unwrap();
    }
    for i in 0..results.times.len() {
        energy_res.write_all(format!("\n{}", results.times[i]).as_bytes()).unwrap();
        for pb in &results.pb_atom.row(i) {
            energy_res.write_all(format!(",{:.3}", pb).as_bytes()).unwrap();
        }
    }
    energy_res.write_all("\n".as_bytes()).unwrap();
    println!("Binding energy terms have been writen to {}", &def_name);
}

fn analyze_sa_res_traj(results: &SMResult, wd: &Path, def_name: &String) {
    println!("Writing binding energy terms...");
    let mut energy_res = fs::File::create(wd.join(&def_name)).unwrap();
    energy_res.write_all("Time (ns)".as_bytes()).unwrap();
    for res in &results.residues {
        energy_res.write_all(format!(",{}#{}", res.nr, res.name).as_bytes()).unwrap();
    }
    for i in 0..results.times.len() {
        energy_res.write_all(format!("\n{}", results.times[i]).as_bytes()).unwrap();
        for sa in &results.sa_atom.row(i) {
            energy_res.write_all(format!(",{:.3}", sa).as_bytes()).unwrap();
        }
    }
    energy_res.write_all("\n".as_bytes()).unwrap();
    println!("Binding energy terms have been writen to {}", &def_name);
}

fn analyze_elec_res_traj(results: &SMResult, wd: &Path, def_name: &String) {
    println!("Writing binding energy terms...");
    let mut energy_res = fs::File::create(wd.join(&def_name)).unwrap();
    energy_res.write_all("Time (ns)".as_bytes()).unwrap();
    for res in &results.residues {
        energy_res.write_all(format!(",{}#{}", res.nr, res.name).as_bytes()).unwrap();
    }
    for i in 0..results.times.len() {
        energy_res.write_all(format!("\n{}", results.times[i]).as_bytes()).unwrap();
        for elec in &results.elec_atom.row(i) {
            energy_res.write_all(format!(",{:.3}", elec).as_bytes()).unwrap();
        }
    }
    energy_res.write_all("\n".as_bytes()).unwrap();
    println!("Binding energy terms have been writen to {}", &def_name);
}

fn analyze_vdw_res_traj(results: &SMResult, wd: &Path, def_name: &String) {
    println!("Writing binding energy terms...");
    let mut energy_res = fs::File::create(wd.join(def_name)).unwrap();
    energy_res.write_all("Time (ns)".as_bytes()).unwrap();
    for res in &results.residues {
        energy_res.write_all(format!(",{}#{}", res.nr, res.name).as_bytes()).unwrap();
    }
    for i in 0..results.times.len() {
        energy_res.write_all(format!("\n{}", results.times[i]).as_bytes()).unwrap();
        for vdw in &results.vdw_atom.row(i) {
            energy_res.write_all(format!(",{:.3}", vdw).as_bytes()).unwrap();
        }
    }
    energy_res.write_all("\n".as_bytes()).unwrap();
    println!("Binding energy terms have been writen to {}", def_name);
}

pub fn output_all_details(results: &SMResult, wd: &Path, sys_name: &String) {
    analyze_dh_res_traj(results, wd, &format!("MMPBSA_{}_res_ΔH.csv", sys_name));
    analyze_mm_res_traj(results, wd, &format!("MMPBSA_{}_res_ΔMM.csv", sys_name));
    analyze_pb_res_traj(results, wd, &format!("MMPBSA_{}_res_ΔPB.csv", sys_name));
    analyze_sa_res_traj(results, wd, &format!("MMPBSA_{}_res_ΔSA.csv", sys_name));
    analyze_elec_res_traj(results, wd, &format!("MMPBSA_{}_res_Δelec.csv", sys_name));
    analyze_vdw_res_traj(results, wd, &format!("MMPBSA_{}_res_ΔvdW.csv", sys_name));
}
