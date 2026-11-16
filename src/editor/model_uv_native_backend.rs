//! Bundled Rust/C++ worker, with no Python environment, packages or downloads.
use serde::{Deserialize,Serialize};
use std::{fs,io::{BufRead,BufReader,Write},path::PathBuf,process::{Child,Command,Stdio},
    sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}},time::{SystemTime,UNIX_EPOCH}};
include!(concat!(env!("OUT_DIR"),"/uv_native_bundle.rs"));
#[derive(Clone,Serialize,Deserialize,Debug)]
pub(super) struct Request {
    pub positions:Vec<[f32;3]>, pub triangles:Vec<[usize;3]>,pub threshold:f32,pub resolution:u32,pub padding:u32,
    #[serde(default,skip_serializing_if="Option::is_none")]pub existing_uv:Option<Vec<[[f32;2];3]>>,
}
#[derive(Clone,Deserialize,Debug)]
pub(super) struct Output {pub uv:Vec<[[f32;2];3]>,pub charts:usize,pub utilization:f32}
#[derive(Default,Clone)]
pub(super) struct Status {pub phase:String,pub fraction:f32}
#[derive(Default)]
pub(super) struct Progress {pub cancelled:AtomicBool,status:Mutex<Status>,child:Mutex<Option<Child>>}
impl Progress {
    pub fn snapshot(&self)->Status {self.status.lock().unwrap_or_else(|e|e.into_inner()).clone()}
    pub fn set(&self,phase:impl Into<String>,fraction:f32){*self.status.lock().unwrap_or_else(|e|e.into_inner())=Status{phase:phase.into(),fraction};}
    pub fn cancel(&self){
        self.cancelled.store(true,Ordering::Relaxed);
        if let Some(mut child)=self.child.lock().unwrap_or_else(|e|e.into_inner()).take(){
            std::thread::spawn(move||{let _=child.kill();let _=child.wait();});
        }
    }
    fn check(&self)->Result<(),String>{if self.cancelled.load(Ordering::Relaxed){Err("Развёртка отменена".into())}else{Ok(())}}
}
impl Drop for Progress {
    fn drop(&mut self){if let Some(mut c)=self.child.get_mut().unwrap_or_else(|e|e.into_inner()).take(){let _=c.kill();let _=c.wait();}}
}
pub(super) fn store()->PathBuf{
    let root=std::env::var_os("BEVY_EDITOR_UV_HOME").map(PathBuf::from).unwrap_or_else(||{
        std::env::var_os("LOCALAPPDATA").or_else(||std::env::var_os("XDG_DATA_HOME")).map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir).join("BevyEditor/uv-runtime")
    });
    if root.is_absolute(){root}else{std::env::current_dir().unwrap_or_else(|_|std::env::temp_dir()).join(root)}
}
fn runtime(progress:&Progress)->Result<PathBuf,String>{
    if let Some(path)=std::env::var_os("BEVY_EDITOR_UV_WORKER").map(PathBuf::from){
        return if path.is_file(){Ok(path)}else{Err("Не найден нативный движок UV".into())};
    }
    use sha2::{Digest,Sha256};
    let mut hash=Sha256::new();for (name,bytes) in NATIVE_BUNDLE{hash.update(name.as_bytes());hash.update(bytes);}
    let root=store().join(format!("native-{:x}",hash.finalize()));
    fs::create_dir_all(&root).map_err(|e|e.to_string())?;
    let mut executable=None;
    for &(name,bytes) in NATIVE_BUNDLE{
        progress.check()?;
        let path=root.join(name);
        if fs::read(&path).ok().as_deref()!=Some(bytes){
            let temporary=root.join(format!("{name}.{}.tmp",std::process::id()));
            fs::write(&temporary,bytes).map_err(|e|e.to_string())?;
            if let Err(e)=fs::rename(&temporary,&path){
                if fs::read(&path).ok().as_deref()!=Some(bytes){return Err(format!("Не удалось подготовить нативный движок UV: {e}"));}
                let _=fs::remove_file(temporary);
            }
        }
        #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;if name=="bevy-uv-native"{fs::set_permissions(&path,fs::Permissions::from_mode(0o755)).map_err(|e|e.to_string())?;}}
        if name=="bevy-uv-native.exe" || name=="bevy-uv-native"{executable=Some(path);}
    }
    executable.ok_or_else(||"В сборке отсутствует нативный движок UV".into())
}
pub(super) fn run(request:Request,progress:Arc<Progress>)->Result<Output,String>{
    progress.check()?;progress.set("Подготовка нативного движка",0.02);
    let worker=runtime(&progress)?;
    let root=store().join("jobs").join(format!("{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()));
    fs::create_dir_all(&root).map_err(|e|e.to_string())?;
    let input=root.join("input.json");let output=root.join("output.json");let log=root.join("worker.log");
    serde_json::to_writer(fs::File::create(&input).map_err(|e|e.to_string())?,&request).map_err(|e|e.to_string())?;
    let mut command=Command::new(worker);
    command.arg(&input).arg(&output).current_dir(&root).stdin(Stdio::null()).stdout(Stdio::piped())
        .stderr(Stdio::from(fs::OpenOptions::new().create(true).append(true).open(&log).map_err(|e|e.to_string())?));
    #[cfg(windows)] {use std::os::windows::process::CommandExt;command.creation_flags(0x08000000);}
    let mut child=command.spawn().map_err(|e|format!("Не удалось запустить нативный движок UV: {e}"))?;
    let stdout=child.stdout.take().ok_or("Нет канала ответа движка")?;
    {let mut owned=progress.child.lock().unwrap_or_else(|e|e.into_inner());
        if progress.cancelled.load(Ordering::Relaxed){let _=child.kill();let _=child.wait();return Err("Развёртка отменена".into());}
        *owned=Some(child);
    }
    let mut log_file=fs::OpenOptions::new().append(true).open(&log).map_err(|e|e.to_string())?;
    for line in BufReader::new(stdout).lines(){
        let line=line.map_err(|e|e.to_string())?;let _=writeln!(log_file,"{line}");
        if let Some(value)=line.strip_prefix("BEVY_UV:").and_then(|s|serde_json::from_str::<serde_json::Value>(s).ok()){
            if let (Some(phase),Some(fraction))=(value["phase"].as_str(),value["fraction"].as_f64()){
                progress.set(phase,(fraction as f32).clamp(0.,1.));
            }
        }
    }
    progress.check()?;
    let mut child=progress.child.lock().unwrap_or_else(|e|e.into_inner()).take().ok_or("Развёртка отменена")?;
    let status=child.wait().map_err(|e|e.to_string())?;
    if !status.success(){return Err(format!("{} · подробности: {}",progress.snapshot().phase,log.display()));}
    progress.check()?;
    if fs::metadata(&output).map_err(|e|e.to_string())?.len()>128*1024*1024{return Err("Ответ движка UV слишком большой".into());}
    let result:Output=serde_json::from_reader(fs::File::open(output).map_err(|e|e.to_string())?).map_err(|e|format!("Повреждён ответ нативного движка UV: {e}"))?;
    if result.charts==0 || result.charts>request.triangles.len() || !result.utilization.is_finite() || !(0.0..=1.00001).contains(&result.utilization)
        ||result.uv.len()!=request.triangles.len() || !result.uv.iter().flatten().flatten().all(|v|v.is_finite() && (-0.00001..=1.00001).contains(v)){
        return Err("UV-карта не прошла проверку. Исходная модель сохранена.".into());
    }
    Ok(result)
}
