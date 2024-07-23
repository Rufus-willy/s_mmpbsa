use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use xdrfile::*;
use crate::settings::Settings;
use crate::utils::resname_3to1;
use ndarray::parallel::prelude::*;
use ndarray::{s, Array1, Array2, Array3, ArrayBase, Axis, Dim, OwnedRepr, ViewRepr};
use std::process::Command;
use std::rc::Rc;
use std::env;
use indicatif::{ProgressBar, ProgressStyle};
use chrono::{Local, Duration};
use crate::coefficients::Coefficients;
use crate::analyzation::Results;
use crate::parse_tpr::Residue;
use crate::apbs_param::{PBASet, PBESet};
use crate::atom_property::{AtomProperties, AtomProperty};
use crate::prepare_apbs::{prepare_pqr, write_apbs_input};

pub fn fun_mmpbsa_calculations(frames: &Vec<Rc<Frame>>, temp_dir: &PathBuf,
                               sys_name: &String, aps: &AtomProperties,
                               ndx_com: &Vec<usize>, ndx_rec: &Vec<usize>, ndx_lig: &Vec<usize>, 
                               ala_list: &Vec<i32>, residues: &Vec<Residue>, 
                               bf: usize, ef: usize, dframe: usize, total_frames: usize,
                               pbe_set: &PBESet, pba_set: &PBASet, settings: &Settings)
                               -> (Results, Vec<Results>) {
    println!("Running MM/PB-SA calculations of {}...", sys_name);
                
    println!("Extracting atoms coordination...");
    let (coordinates, _) = get_atoms_trj(&frames);   // frames x atoms(3x1)
    let time_list: Vec<f32> = frames.iter().map(|f| f.time / 1000.0).collect();

    // calculate MM and PBSA
    let result_wt = calculate_mmpbsa(&time_list, &coordinates, "WT", bf, ef, dframe, 
        total_frames, aps, &temp_dir, &ndx_com, &ndx_rec, &ndx_lig, residues,
        sys_name, pbe_set, pba_set, settings);

    let mut result_ala_scan: Vec<Results> = vec![];
    if ala_list.len() > 0 {
        // main chain atoms number
        let as_res: Vec<&Residue> = residues.iter().filter(|&r| ala_list.contains(&r.nr) && r.name.ne("GLY")).collect();
        for asr in as_res {
            let mut new_aps = aps.clone();
            let as_atoms: Vec<AtomProperty> = aps.atom_props.iter().filter_map(|a| if a.resid == asr.id {
                Some(a.clone())
            } else {
                None
            }).collect();
            let mut sc_out: Vec<&AtomProperty> = as_atoms.iter().filter(|&a| a.name.ne("N") && a.name.ne("HN") 
                && a.name.ne("CB") && a.name.ne("HCB") && a.name.ne("CA") && a.name.ne("HCA") 
                && a.name.ne("C") && a.name.ne("O")).collect();
            let xgs: Vec<AtomProperty> = as_atoms.iter().filter_map(|a| {
                if a.name.eq("CG") || a.name.eq("SG") || a.name.eq("OG") {
                    Some(a.clone())
                } else {
                    None
                }}).collect();
            for xg in xgs.iter() {
                new_aps.atom_props[xg.id].change_atom(aps.at_map["HC"], "HB", &aps.radius_type);
            }
            // 脯氨酸需要把CD改成H
            if asr.name.eq("PRO") {
                sc_out.retain(|&a| a.name.ne("CD"));
                let cg = sc_out.iter().find(|&&a| a.name == "CG").unwrap();
                new_aps.atom_props[cg.id].change_atom(aps.at_map["HN"], "HN", &aps.radius_type);
            }
            
            // delete other atoms in the scanned residue
            let del_list: Vec<usize> = sc_out.iter().map(|a| a.id).collect();
            let xg_list: Vec<usize> = xgs.iter().map(|a| a.id).collect();
            new_aps.atom_props.retain(|a| !del_list.contains(&a.id) || xg_list.contains(&a.id));
            let retain_id: Vec<usize> = new_aps.atom_props.iter().map(|a| a.id).collect();
            for (i, ap) in new_aps.atom_props.iter_mut().enumerate() {
                ap.id = i;
            };
            let new_coordinates = coordinates.select(Axis(1), &retain_id);
            let new_ndx_com = Vec::from_iter(0..(ndx_com.len() - (aps.atom_props.len() - new_aps.atom_props.len())));
            let new_ndx_rec = match ndx_rec[0].partial_cmp(&ndx_lig[0]) {
                Some(Ordering::Less) => Vec::from_iter(0..(ndx_rec.len() - (aps.atom_props.len() - new_aps.atom_props.len()))),
                Some(Ordering::Greater) => Vec::from_iter(ndx_lig.len()..(ndx_com.len() - (aps.atom_props.len() - new_aps.atom_props.len()))),
                Some(Ordering::Equal) => new_ndx_com.to_vec(),
                None => vec![]
            };
            let new_ndx_lig = match ndx_rec[0].partial_cmp(&ndx_lig[0]) {
                Some(Ordering::Less) => Vec::from_iter(new_ndx_rec.len()..new_ndx_com.len()),
                Some(Ordering::Greater) => Vec::from_iter(0..ndx_lig.len()),
                Some(Ordering::Equal) => new_ndx_com.to_vec(),
                None => vec![]
            };

            // After alanine mutation
            if let Some(mutation) = resname_3to1(&asr.name) {
                let result_as = calculate_mmpbsa(&time_list, &new_coordinates, &format!("{}{}A", mutation, asr.nr),
                    bf, ef, dframe, total_frames, &new_aps, &temp_dir, 
                    &new_ndx_com, &new_ndx_rec, &new_ndx_lig, residues,
                    sys_name, pbe_set, pba_set, settings);
                result_ala_scan.push(result_as);
            } else {
                println!("Residue unknown: {}", asr.name);
            }
        }
    };

    // whether remove temp directory
    if !settings.debug_mode {
        if settings.apbs.is_some() {
            fs::remove_dir_all(&temp_dir).expect("Remove dir failed");
        }
    }

    (result_wt, result_ala_scan)
}

fn get_atoms_trj(frames: &Vec<Rc<Frame>>) -> (Array3<f64>, Array3<f64>) {
    let num_frames = frames.len();
    let num_atoms = frames[0].num_atoms();
    let mut coord_matrix: Array3<f64> = Array3::zeros((num_frames, num_atoms, 3));
    let mut box_size: Array3<f64> = Array3::zeros((num_frames, 3, 3));
    let pb = ProgressBar::new(frames.len() as u64);
    set_style(&pb);
    for (idx, frame) in frames.into_iter().enumerate() {
        for (i, a) in (&frame.coords).into_iter().enumerate() {
            for j in 0..3 {
                coord_matrix[[idx, i, j]] = a[j] as f64 * 10.0;
            }
        }
        for (i, b) in (&frame.box_vector).into_iter().enumerate() {
            for j in 0..3 {
                box_size[[idx, i, j]] = b[j] as f64 * 10.0;
            }
        }
        pb.inc(1);
    }
    pb.finish();
    return (coord_matrix, box_size);
}

pub fn set_style(pb: &ProgressBar) {
    pb.set_style(ProgressStyle::with_template(
        "[{elapsed_precise}] {bar:50.cyan/cyan} {pos}/{len} {msg}").unwrap()
        .progress_chars("=>-"));
}

fn calculate_mmpbsa(time_list: &Vec<f32>, coordinates: &Array3<f64>, mutation: &str, bf: usize, ef: usize, 
                    dframe: usize, total_frames: usize, aps: &AtomProperties, temp_dir: &PathBuf,
                    ndx_com_norm: &Vec<usize>, ndx_rec_norm: &Vec<usize>, ndx_lig_norm: &Vec<usize>,
                    residues: &Vec<Residue>, sys_name: &String, 
                    pbe_set: &PBESet, pba_set: &PBASet, settings: &Settings) -> Results {
    let mut elec_res: Array2<f64> = Array2::zeros((total_frames, residues.len()));
    let mut vdw_res: Array2<f64> = Array2::zeros((total_frames, residues.len()));
    let mut pb_res: Array2<f64> = Array2::zeros((total_frames, residues.len()));
    let mut sa_res: Array2<f64> = Array2::zeros((total_frames, residues.len()));
    
    // parameters for elec calculation
    let coeff = Coefficients::new(pbe_set);

    // Time list of trajectory
    let times: Array1<f64> = Array1::from_iter((bf..=ef).into_iter().step_by(dframe).map(|f| time_list[f] as f64));

    // start calculation
    env::set_var("OMP_NUM_THREADS", settings.nkernels.to_string());
    let t_start = Local::now();
    
    println!("Calculating MM/PB-SA binding energy...");

    let pgb = ProgressBar::new(total_frames as u64);
    set_style(&pgb);
    pgb.inc(0);
    let mut idx = 0;
    pgb.set_message(format!("at {} ns...", times[idx]));
    for cur_frm in (bf..=ef).step_by(dframe) {
        // MM
        let coord = coordinates.slice(s![cur_frm, .., ..]);
        if ndx_lig_norm[0] != ndx_rec_norm[0] {
            let (res_elec, res_vdw) = 
                calc_mm(&ndx_rec_norm, &ndx_lig_norm, aps, &coord, residues, &coeff, &settings);
            elec_res.row_mut(idx).assign(&res_elec);
            vdw_res.row_mut(idx).assign(&res_vdw);
        }

        // PBSA
        if settings.apbs.is_some() {
            prepare_pqr(cur_frm, &time_list, &temp_dir, sys_name, &coordinates, ndx_com_norm, &ndx_rec_norm, ndx_lig_norm, aps);
            calc_pbsa(idx, &coord, time_list, ndx_rec_norm, ndx_lig_norm, ndx_com_norm,
                &mut pb_res, &mut sa_res, cur_frm, sys_name, temp_dir, aps, pbe_set, pba_set, settings);
        }

        pgb.inc(1);
        pgb.set_message(format!("at {} ns, ΔH={:.2} kJ/mol, eta. {} s", 
                                        times[idx],
                                        vdw_res.row(idx).sum() + elec_res.row(idx).sum() + pb_res.row(idx).sum() + sa_res.row(idx).sum(),
                                        pgb.eta().as_secs()));

        idx += 1;
    }
    pgb.finish();

    // end calculation
    let t_end = Local::now();
    let t_spend = Duration::from(t_end - t_start).num_milliseconds();
    println!("MM/PB-SA calculation of {} finished. Total time cost: {} s", sys_name, t_spend as f64 / 1000.0);
    env::remove_var("OMP_NUM_THREADS");

    Results::new(
        aps,
        residues,
        ndx_lig_norm,
        &times,
        coordinates.clone(),
        mutation,
        &elec_res,
        &vdw_res,
        &pb_res,
        &sa_res,
    )
}

fn calc_mm(ndx_rec_norm: &Vec<usize>, ndx_lig_norm: &Vec<usize>, aps: &AtomProperties, coord: &ArrayBase<ViewRepr<&f64>, Dim<[usize; 2]>>, 
            residues: &Vec<Residue>, coeff: &Coefficients, settings: &Settings) -> (Array1<f64>, Array1<f64>) {
    let kj_elec = coeff.kj_elec;
    let kap = coeff.kap;
    let pdie = coeff.pdie;
    let mut de_elec: Array1<f64> = Array1::zeros(residues.len());
    let mut de_vdw: Array1<f64> = Array1::zeros(residues.len());

    for &i in ndx_rec_norm {
        let qi = aps.atom_props[i].charge;
        let ci = aps.atom_props[i].type_id;
        let xi = coord[[i, 0]];
        let yi = coord[[i, 1]];
        let zi = coord[[i, 2]];
        for &j in ndx_lig_norm {
            if ndx_lig_norm[0] == ndx_rec_norm[0] && j <= i {
                continue;
            }
            let qj = aps.atom_props[j].charge;
            let cj = aps.atom_props[j].type_id;
            let xj = coord[[j, 0]];
            let yj = coord[[j, 1]];
            let zj = coord[[j, 2]];
            let r = f64::sqrt((xi - xj).powi(2) + (yi - yj).powi(2) + (zi - zj).powi(2));
            if r < settings.r_cutoff {
                let e_elec = match settings.use_dh {
                    false => qi * qj / r,
                    true => qi * qj / r * f64::exp(-kap * r)   // doi: 10.1088/0256-307X/38/1/018701
                }; // use A
                let r = r / 10.0;
                let e_vdw = if aps.c10[[ci, cj]] < 1e-10 {
                    (aps.c12[[ci, cj]] / r.powi(6) - aps.c6[[ci, cj]]) / r.powi(6) // use nm
                } else {
                    // use 12-10 style to calculate LJ for pdbqt hbond
                    aps.c12[[ci, cj]] / r.powi(12) - aps.c10[[ci, cj]] / r.powi(10)
                };
                de_elec[aps.atom_props[i].resid] += e_elec;
                de_elec[aps.atom_props[j].resid] += e_elec;
                de_vdw[aps.atom_props[i].resid] += e_vdw;
                de_vdw[aps.atom_props[j].resid] += e_vdw;
            }
        }
    }

    de_elec.par_iter_mut().for_each(|p| *p *= kj_elec / (2.0 * pdie));
    de_vdw.par_iter_mut().for_each(|p| *p /= 2.0);

    return (de_elec, de_vdw)
}

fn calc_pbsa(idx: usize, coord: &ArrayBase<ViewRepr<&f64>, Dim<[usize; 2]>>, time_list: &Vec<f32>, 
            ndx_rec_norm: &Vec<usize>, ndx_lig_norm: &Vec<usize>, ndx_com_norm: &Vec<usize>,
            pb_res: &mut ArrayBase<OwnedRepr<f64>, Dim<[usize; 2]>>, sa_res: &mut ArrayBase<OwnedRepr<f64>, Dim<[usize; 2]>>,
            cur_frm: usize, sys_name: &String, temp_dir: &PathBuf, 
            aps: &AtomProperties, pbe_set: &PBESet, pba_set: &PBASet, settings: &Settings) {
    // From AMBER-PB4, the surface extension constant γ=0.0072 kcal/(mol·Å2)=0.030125 kJ/(mol·Å^2)
    // but the default gamma parameter for apbs calculation is set to 1, in order to directly obtain the surface area
    // then the SA energy term is calculated by s_mmpbsa
    let gamma = 0.030125;
    let bias = 0.0;
    let f_name = format!("{}_{}ns", sys_name, time_list[cur_frm]);
    if let Some(apbs) = &settings.apbs {
        write_apbs_input(ndx_rec_norm, ndx_lig_norm, coord, &Array1::from_iter(aps.atom_props.iter().map(|a| a.radius)),
                pbe_set, pba_set, temp_dir, &f_name, settings);
        // invoke apbs program to do apbs calculations
        let apbs_result = Command::new(apbs).arg(format!("{}.apbs", f_name)).current_dir(temp_dir).output().expect("running apbs failed.");
        let apbs_err = String::from_utf8(apbs_result.stderr).expect("Failed to parse apbs output.");
        let apbs_result = String::from_utf8(apbs_result.stdout).expect("Failed to parse apbs output.");
        if settings.debug_mode {
            let mut outfile = File::create(temp_dir.join(format!("{}.out", f_name))).expect("Failed to create output file.");
            outfile.write_all(apbs_result.as_bytes()).expect("Failed to write apbs output.");
            let mut errfile = File::create(temp_dir.join(format!("{}.err", f_name))).expect("Failed to create err file.");
            errfile.write_all(apbs_err.as_bytes()).expect("Failed to write apbs output.");
        }
        // let apbs_result = fs::read_to_string(temp_dir.join(format!("{}.out", f_name))).expect("Failed to parse apbs output.");

        // preserve CALCULATION, Atom and SASA lines
        let apbs_result: Vec<&str> = apbs_result.split("\n").filter_map(|p|
            if p.trim().starts_with("CALCULATION") || p.trim().starts_with("Atom") || p.trim().starts_with("SASA") {
                Some(p.trim())
            } else {
                None
            }
        ).collect();

        // extract apbs results
        let indexes: Vec<usize> = apbs_result.iter().enumerate().filter_map(|(i, &p)| match p.starts_with("CAL") {
            true => Some(i),
            false => None
        }).collect();

        let mut com_pb_sol: Vec<f64> = vec![];
        let mut com_pb_vac: Vec<f64> = vec![];
        let mut rec_pb_sol: Vec<f64> = vec![];
        let mut rec_pb_vac: Vec<f64> = vec![];
        let mut lig_pb_sol: Vec<f64> = vec![];
        let mut lig_pb_vac: Vec<f64> = vec![];
        let mut com_sa: Vec<f64> = vec![];
        let mut rec_sa: Vec<f64> = vec![];
        let mut lig_sa: Vec<f64> = vec![];

        let mut skip_pb = true;     // the first time PB calculation should be wasted
        for (i, &idx) in indexes.iter().enumerate() {
            let st = idx + 1;
            let ed = match indexes.get(i + 1) {
                Some(&idx) => idx,
                None => apbs_result.len()
            };
            if apbs_result[idx].contains(&"_com_SOL") {
                if !skip_pb {
                    apbs_result[st..ed].par_iter().map(|&p| parse_apbs_line(p)).collect_into_vec(&mut com_pb_sol);
                }
                skip_pb = !skip_pb;
            } else if apbs_result[idx].contains(&"_com_VAC") {
                if !skip_pb {
                    apbs_result[st..ed].par_iter().map(|&p| parse_apbs_line(p)).collect_into_vec(&mut com_pb_vac);
                }
                skip_pb = !skip_pb;
            } else if apbs_result[idx].contains(&"_rec_SOL") {
                if !skip_pb {
                    apbs_result[st..ed].par_iter().map(|&p| parse_apbs_line(p)).collect_into_vec(&mut rec_pb_sol);
                }
                skip_pb = !skip_pb;
            } else if apbs_result[idx].contains(&"_rec_VAC") {
                if !skip_pb {
                    apbs_result[st..ed].par_iter().map(|&p| parse_apbs_line(p)).collect_into_vec(&mut rec_pb_vac);
                }
                skip_pb = !skip_pb;
            } else if apbs_result[idx].contains(&"_lig_SOL") {
                if !skip_pb {
                    apbs_result[st..ed].par_iter().map(|&p| parse_apbs_line(p)).collect_into_vec(&mut lig_pb_sol);
                }
                skip_pb = !skip_pb;
            } else if apbs_result[idx].contains(&"_lig_VAC") {
                if !skip_pb {
                    apbs_result[st..ed].par_iter().map(|&p| parse_apbs_line(p)).collect_into_vec(&mut lig_pb_vac);
                }
                skip_pb = !skip_pb;
            } else if apbs_result[idx].contains(&"_com_SAS") {
                apbs_result[st..ed].par_iter().map(|&p| parse_apbs_line(p)).collect_into_vec(&mut com_sa);
            } else if apbs_result[idx].contains(&"_rec_SAS") {
                apbs_result[st..ed].par_iter().map(|&p| parse_apbs_line(p)).collect_into_vec(&mut rec_sa);
            } else if apbs_result[idx].contains(&"_lig_SAS") {
                apbs_result[st..ed].par_iter().map(|&p| parse_apbs_line(p)).collect_into_vec(&mut lig_sa);
            }
        }

        let com_pb: Array1<f64> = Array1::from_vec(com_pb_sol) - Array1::from_vec(com_pb_vac);
        let com_sa: Array1<f64> = Array1::from_vec(com_sa.par_iter().map(|i| gamma * *i + bias / com_sa.len() as f64).collect());
        let rec_pb: Array1<f64> = Array1::from_vec(rec_pb_sol) - Array1::from_vec(rec_pb_vac);
        let rec_sa: Array1<f64> = Array1::from_vec(rec_sa.par_iter().map(|i| gamma * *i + bias / rec_sa.len() as f64).collect());
        let lig_pb: Array1<f64> = Array1::from_vec(lig_pb_sol) - Array1::from_vec(lig_pb_vac);
        let lig_sa: Array1<f64> = Array1::from_vec(lig_sa.par_iter().map(|i| gamma * *i + bias / lig_sa.len() as f64).collect());

        // residue decomposition
        let offset_rec = match ndx_lig_norm[0].cmp(&ndx_rec_norm[0]) {
            Ordering::Less => 0,
            Ordering::Greater => ndx_rec_norm.len(),
            Ordering::Equal => 0
        };
        let offset_lig = match ndx_rec_norm[0].cmp(&ndx_lig_norm[0]) {
            Ordering::Less => 0,
            Ordering::Greater => ndx_lig_norm.len(),
            Ordering::Equal => 0
        };

        if ndx_rec_norm[0] == ndx_lig_norm[0] {
            // if no ligand, pb_com = pb_lig = 0, so real energy is inversed rec_pbsa
            for &i in ndx_com_norm {
                pb_res[[idx, aps.atom_props[i].resid]] += rec_pb[i - offset_lig];
                sa_res[[idx, aps.atom_props[i].resid]] += rec_sa[i - offset_lig];
            }
        } else {
            for &i in ndx_com_norm {
                if ndx_rec_norm.contains(&i) {
                    pb_res[[idx, aps.atom_props[i].resid]] += com_pb[i] - rec_pb[i - offset_lig];
                    sa_res[[idx, aps.atom_props[i].resid]] += com_sa[i] - rec_sa[i - offset_lig];
                } else {
                    pb_res[[idx, aps.atom_props[i].resid]] += com_pb[i] - lig_pb[i - offset_rec];
                    sa_res[[idx, aps.atom_props[i].resid]] += com_sa[i] - lig_sa[i - offset_rec];
                }
            }
        }
    }
}

fn parse_apbs_line(line: &str) -> f64 {
    line.split(":")
        .skip(1)
        .next().expect("Cannot get information from apbs")
        .trim_start()
        .split(" ")
        .next().expect("Cannot get information from apbs")
        .parse().expect("Cannot parse value from apbs")
}