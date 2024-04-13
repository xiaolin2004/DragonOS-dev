use crate::driver::base::device::bus::Bus;

use crate::driver::base::device::driver::{Driver, DriverCommonData};
use crate::driver::base::device::{Device, IdTable};
use crate::driver::base::kobject::{KObjType, KObject, KObjectCommonData, KObjectState, LockedKObjectState};
use crate::driver::base::kset::KSet;

use crate::filesystem::kernfs::KernFSInode;

use crate::init::initcall::INITCALL_DEVICE;
use crate::libs::rwlock::{RwLockReadGuard, RwLockWriteGuard};
use crate::libs::spinlock::SpinLockGuard;
use crate::libs::spinlock::SpinLock;
use alloc::string::ToString;

use alloc::sync::Weak;
use alloc::{string::String, sync::Arc, vec::Vec};
use system_error::SystemError;
use unified_init::macros::unified_init;

use core::fmt::Debug;


#[derive(Debug)]
#[cast_to([sync] Driver)]
pub struct AhciDiskDriver{
    inner:SpinLock<InnerAhciDiskDriver>,
    locked_kobjstate:LockedKObjectState,
}

impl AhciDiskDriver{
    const NAME:&str = "ahci_disk";

    fn new() -> Arc<Self> {
        Arc::new(AhciDiskDriver {
            inner: SpinLock::new(InnerAhciDiskDriver {
                driver_common: DriverCommonData::default(),
                kobject_common: KObjectCommonData::default(),
            }),
            locked_kobjstate: LockedKObjectState::new(None),
        })
    }

    fn inner(&self) -> SpinLockGuard<InnerAhciDiskDriver> {
        self.inner.lock()
    }

}

#[derive(Debug)]
pub struct InnerAhciDiskDriver{
    driver_common: DriverCommonData,
    kobject_common: KObjectCommonData,
}


impl Driver for AhciDiskDriver{
    fn id_table(&self) -> Option<IdTable> {
        Some(IdTable::new(Self::NAME.to_string(), None))
    }

    fn devices(&self) -> Vec<Arc<dyn Device>> {
        self.inner().driver_common.devices.clone()
    }

    fn add_device(&self, device: Arc<dyn Device>) {
        self.inner().driver_common.push_device(device);
    }

    fn delete_device(&self, device: &Arc<dyn Device>) {
        self.inner().driver_common.delete_device(device);
    }

    fn set_bus(&self, bus: Option<Weak<dyn Bus>>) {
        self.inner().driver_common.bus = bus;
    }

    fn bus(&self) -> Option<Weak<dyn Bus>> {
        self.inner().driver_common.bus.clone()
    }
}

impl KObject for AhciDiskDriver{
    fn as_any_ref(&self) -> &dyn core::any::Any {
        self
    }

    fn set_inode(&self, inode: Option<Arc<KernFSInode>>) {
        self.inner().kobject_common.kern_inode = inode;
    }

    fn inode(&self) -> Option<Arc<KernFSInode>> {
        self.inner().kobject_common.kern_inode.clone()
    }

    fn parent(&self) -> Option<Weak<dyn KObject>> {
        self.inner().kobject_common.parent.clone()
    }

    fn set_parent(&self, parent: Option<Weak<dyn KObject>>) {
        self.inner().kobject_common.parent = parent;
    }

    fn kset(&self) -> Option<Arc<KSet>> {
        self.inner().kobject_common.kset.clone()
    }

    fn set_kset(&self, kset: Option<Arc<KSet>>) {
        self.inner().kobject_common.kset = kset;
    }

    fn kobj_type(&self) -> Option<&'static dyn KObjType> {
        self.inner().kobject_common.kobj_type
    }

    fn set_kobj_type(&self, ktype: Option<&'static dyn KObjType>) {
        self.inner().kobject_common.kobj_type = ktype;
    }

    fn name(&self) -> String {
        Self::NAME.to_string()
    }

    fn set_name(&self, _name: String) {
        // do nothing
    }

    fn kobj_state(&self) -> RwLockReadGuard<KObjectState> {
        self.locked_kobjstate.read()
    }

    fn kobj_state_mut(&self) -> RwLockWriteGuard<KObjectState> {
        self.locked_kobjstate.write()
    }

    fn set_kobj_state(&self, state: KObjectState) {
        *self.locked_kobjstate.write() = state;
    }
}

#[unified_init(INITCALL_DEVICE)]
pub fn ahci_driver_init()->Result<(),SystemError>{
    let driver=AhciDiskDriver::new();
    // TODO 等待PCI接入总线管理
    Ok(())
    
}