use crate::debugger::debugee::dwarf::{DebugeeContext, EndianRcSlice};
use crate::debugger::debugee::flow::{ControlFlow, DebugeeEvent};
use crate::debugger::debugee::rendezvous::Rendezvous;
use crate::debugger::debugee::thread::ThreadCtl;
use crate::debugger::GlobalAddress;
use anyhow::anyhow;
use log::{info, warn};
use nix::unistd::Pid;
use object::{Object, ObjectSection};
use proc_maps::MapRange;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub mod dwarf;
pub mod flow;
mod rendezvous;
pub mod thread;

/// Debugee - represent static and runtime debugee information.
pub struct Debugee {
    /// true if debugee currently start.
    pub in_progress: bool,
    /// path to debugee file.
    pub path: PathBuf,
    /// debugee process map address.
    pub mapping_addr: Option<usize>,
    /// preparsed debugee dwarf.
    pub dwarf: DebugeeContext<EndianRcSlice>,
    /// debugee control flow
    pub control_flow: ControlFlow,
    /// elf file sections (name => address).
    object_sections: HashMap<String, u64>,
    /// rendezvous struct maintained by dyn linker.
    rendezvous: Option<Rendezvous>,
}

impl Debugee {
    pub fn new_non_running<'a, 'b, OBJ>(
        path: &Path,
        proc: Pid,
        object: &'a OBJ,
    ) -> anyhow::Result<Self>
    where
        'a: 'b,
        OBJ: Object<'a, 'b>,
    {
        let dwarf_builder = dwarf::DebugeeContextBuilder::default();
        Ok(Self {
            in_progress: false,
            path: path.into(),
            mapping_addr: None,
            //  threads_ctl: thread::ThreadCtl::new(proc),
            dwarf: dwarf_builder.build(object)?,
            control_flow: ControlFlow::new(proc, GlobalAddress(object.entry() as usize)),
            object_sections: object
                .sections()
                .filter_map(|section| Some((section.name().ok()?.to_string(), section.address())))
                .collect(),
            rendezvous: None,
        })
    }

    /// Return debugee process mapping offset.
    /// This method will panic if called before debugee started,
    /// calling a method on time is the responsibility of the caller.
    pub fn mapping_offset(&self) -> usize {
        self.mapping_addr.expect("mapping address must exists")
    }

    /// Return rendezvous struct.
    /// This method will panic if called before program entry point evaluated,
    /// calling a method on time is the responsibility of the caller.
    #[allow(unused)]
    pub fn rendezvous(&self) -> &Rendezvous {
        self.rendezvous.as_ref().expect("rendezvous must exists")
    }

    fn init_libthread_db(&mut self) {
        match self.control_flow.threads_ctl.init_thread_db() {
            Ok(_) => {
                info!("libthread_db enabled")
            }
            Err(e) => {
                warn!(
                    "libthread_db load fail with \"{e}\", some thread debug functions are omitted"
                );
            }
        }
    }

    pub fn control_flow_tick(&mut self) -> anyhow::Result<DebugeeEvent> {
        let event = self.control_flow.tick(self.mapping_addr)?;
        match event {
            DebugeeEvent::DebugeeExit(_) => {
                self.in_progress = false;
            }
            DebugeeEvent::DebugeeStart => {
                self.in_progress = true;
                self.mapping_addr = Some(self.define_mapping_addr()?);
            }
            flow::DebugeeEvent::AtEntryPoint(tid) => {
                self.rendezvous = Some(Rendezvous::new(
                    tid,
                    self.mapping_offset(),
                    &self.object_sections,
                )?);
                self.init_libthread_db();
            }
            _ => {}
        }

        Ok(event)
    }

    pub fn threads_ctl(&self) -> &ThreadCtl {
        &self.control_flow.threads_ctl
    }

    fn define_mapping_addr(&mut self) -> anyhow::Result<usize> {
        let absolute_debugee_path_buf = self.path.canonicalize()?;
        let absolute_debugee_path = absolute_debugee_path_buf.as_path();

        let proc_maps: Vec<MapRange> =
            proc_maps::get_process_maps(self.threads_ctl().proc_pid().as_raw())?
                .into_iter()
                .filter(|map| map.filename() == Some(absolute_debugee_path))
                .collect();

        let lowest_map = proc_maps
            .iter()
            .min_by(|map1, map2| map1.start().cmp(&map2.start()))
            .ok_or_else(|| anyhow!("mapping not found"))?;

        Ok(lowest_map.start())
    }
}
