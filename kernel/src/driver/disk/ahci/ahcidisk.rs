use alloc::string::ToString;
use alloc::sync::Weak;
use alloc::{string::String, sync::Arc, vec::Vec};
use core::any::Any;
use core::default::Default as Default1;
use core::fmt::Debug;
use core::sync::atomic::{compiler_fence, Ordering};
use core::{mem::size_of, ptr::write_bytes};

use log::error;

use intertrait::CastFromSync;
use system_error::SystemError;

use crate::arch::mm::kernel_page_flags;
use crate::arch::MMArch;
use crate::driver::base::block::block_device::{BlockDevice, BlockId};
use crate::driver::base::block::disk_info::Partition;
use crate::driver::base::class::Class;
use crate::driver::base::device::bus::Bus;
use crate::driver::base::device::driver::{Driver, DriverCommonData, DriverProbeType};
use crate::driver::base::device::{Device, DeviceCommonData, DeviceType, IdTable};
use crate::driver::base::kobject::{
    KObjType, KObject, KObjectCommonData, KObjectState, LockedKObjectState,
};
use crate::driver::base::kset::KSet;
use crate::driver::disk::ahci::hba::{
    FisRegH2D, FisType, HbaCmdHeader, ATA_CMD_READ_DMA_EXT, ATA_CMD_WRITE_DMA_EXT, ATA_DEV_BUSY,
    ATA_DEV_DRQ,
};
use crate::driver::disk::ahci::HBA_PxIS_TFES;
use crate::driver::pci::attr::DeviceID;
use crate::exception::IrqNumber;
use crate::filesystem::kernfs::KernFSInode;
use crate::filesystem::mbr::MbrDiskPartionTable;
use crate::filesystem::sysfs::AttributeGroup;
use crate::ipc::shm::ShmCtlCmd::Default;
use crate::libs::rwlock::{RwLockReadGuard, RwLockWriteGuard};
use crate::libs::spinlock::{SpinLock, SpinLockGuard};
use crate::mm::{verify_area, MemoryManagementArch, PhysAddr, VirtAddr};

use super::{_port, hba::HbaCmdTable};

/// @brief: 只支持MBR分区格式的磁盘结构体
pub struct AhciDisk {
    pub disk_private: SpinLock<DiskPrivateData>,
    dev_id: Arc<DeviceID>,
    inner: SpinLock<InnerAhciDiskDevice>,
    locked_kobject_state: LockedKObjectState,

    /// 指向LockAhciDisk的弱引用
    self_ref: Weak<AhciDisk>,
}

impl AhciDisk {
    pub fn new(
        name: String,
        flags: u16,
        ctrl_num: u8,
        port_num: u8,
    ) -> Result<Arc<Self>, SystemError> {
        let dev = Arc::new_cyclic(|self_ref| Self {
            self_ref: self_ref.clone(),
            locked_kobject_state: LockedKObjectState::default(),
            inner: SpinLock::new(InnerAhciDiskDevice {
                device_common: DeviceCommonData::default(),
                kobject_common: KObjectCommonData::default(),
                irq: None,
                name: None,
            }),
            disk_private: SpinLock::new(DiskPrivateData {
                name,
                flags,
                ctrl_num,
                port_num,
                partitions: Default1::default(),
            }),
            dev_id: Arc::new(DeviceID),
        });
        let table = dev.read_mbr_table()?;
        let partitions = table.partitions(Arc::downgrade(&dev) as Weak<dyn BlockDevice>);
        dev.private().partitions = partitions;

        Ok(dev)
    }
    fn inner(&self) -> SpinLockGuard<InnerAhciDiskDevice> {
        self.inner.lock()
    }
    pub fn private(&self) -> SpinLockGuard<DiskPrivateData> {
        self.disk_private.lock()
    }

    pub fn read_mbr_table(&self) -> Result<MbrDiskPartionTable, SystemError> {
        let disk = self.self_ref.upgrade().unwrap() as Arc<dyn BlockDevice>;
        MbrDiskPartionTable::from_disk(disk)
    }
}

pub struct DiskPrivateData {
    pub name: String,
    pub flags: u16,                      // 磁盘的状态flags
    pub partitions: Vec<Arc<Partition>>, // 磁盘分区数组
    // port: &'static mut HbaPort,      // 控制硬盘的端口
    pub ctrl_num: u8,
    pub port_num: u8,
}

struct InnerAhciDiskDevice {
    device_common: DeviceCommonData,
    kobject_common: KObjectCommonData,
    irq: Option<IrqNumber>,
    name: Option<String>,
}

const AHCI_DISK_BASENAME: &str = "ahci_disk";

/// 函数实现
impl Debug for AhciDisk {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{{ name: {}, flags: {}, part_s: {:?} }}",
            self.private().name,
            self.private().flags,
            self.private().partitions
        )?;
        return Ok(());
    }
}

impl AhciDisk {
    fn read_at(
        &self,
        lba_id_start: BlockId, // 起始lba编号
        count: usize,          // 读取lba的数量
        buf: &mut [u8],
    ) -> Result<usize, SystemError> {
        assert!((buf.len() & 511) == 0);
        compiler_fence(Ordering::SeqCst);
        let check_length = ((count - 1) >> 4) + 1; // prdt length
        if count * 512 > buf.len() || check_length > 8_usize {
            error!("ahci read: e2big");
            // 不可能的操作
            return Err(SystemError::E2BIG);
        } else if count == 0 {
            return Ok(0);
        }

        let port = _port(self.private().ctrl_num, self.private().port_num);
        volatile_write!(port.is, u32::MAX); // Clear pending interrupt bits

        let slot = port.find_cmdslot().unwrap_or(u32::MAX);

        if slot == u32::MAX {
            return Err(SystemError::EIO);
        }

        #[allow(unused_unsafe)]
        let cmdheader: &mut HbaCmdHeader = unsafe {
            (MMArch::phys_2_virt(PhysAddr::new(
                volatile_read!(port.clb) as usize + slot as usize * size_of::<HbaCmdHeader>(),
            ))
            .unwrap()
            .data() as *mut HbaCmdHeader)
                .as_mut()
                .unwrap()
        };

        cmdheader.cfl = (size_of::<FisRegH2D>() / size_of::<u32>()) as u8;

        volatile_set_bit!(cmdheader.cfl, 1 << 6, false); //  Read/Write bit : Read from device
        volatile_write!(cmdheader.prdtl, check_length as u16); // PRDT entries count

        // 设置数据存放地址
        let mut buf_ptr = buf as *mut [u8] as *mut usize as usize;

        // 由于目前的内存管理机制无法把用户空间的内存地址转换为物理地址，所以只能先把数据拷贝到内核空间
        // TODO：在内存管理重构后，可以直接使用用户空间的内存地址

        let user_buf = verify_area(VirtAddr::new(buf_ptr), buf.len()).is_ok();
        let mut kbuf = if user_buf {
            let x: Vec<u8> = vec![0; buf.len()];
            Some(x)
        } else {
            None
        };

        if kbuf.is_some() {
            buf_ptr = kbuf.as_mut().unwrap().as_mut_ptr() as usize;
        }

        #[allow(unused_unsafe)]
        let cmdtbl = unsafe {
            (MMArch::phys_2_virt(PhysAddr::new(volatile_read!(cmdheader.ctba) as usize))
                .unwrap()
                .data() as *mut HbaCmdTable)
                .as_mut()
                .unwrap() // 必须使用 as_mut ，得到的才是原来的变量
        };
        let mut tmp_count = count;

        unsafe {
            // 清空整个table的旧数据
            write_bytes(cmdtbl, 0, 1);
        }
        // debug!("cmdheader.prdtl={}", volatile_read!(cmdheader.prdtl));

        // 8K bytes (16 sectors) per PRDT
        for i in 0..((volatile_read!(cmdheader.prdtl) - 1) as usize) {
            volatile_write!(
                cmdtbl.prdt_entry[i].dba,
                MMArch::virt_2_phys(VirtAddr::new(buf_ptr)).unwrap().data() as u64
            );
            cmdtbl.prdt_entry[i].dbc = 8 * 1024 - 1;
            volatile_set_bit!(cmdtbl.prdt_entry[i].dbc, 1 << 31, true); // 允许中断 prdt_entry.i
            buf_ptr += 8 * 1024;
            tmp_count -= 16;
        }

        // Last entry
        let las = (volatile_read!(cmdheader.prdtl) - 1) as usize;
        volatile_write!(
            cmdtbl.prdt_entry[las].dba,
            MMArch::virt_2_phys(VirtAddr::new(buf_ptr)).unwrap().data() as u64
        );
        cmdtbl.prdt_entry[las].dbc = ((tmp_count << 9) - 1) as u32; // 数据长度

        volatile_set_bit!(cmdtbl.prdt_entry[las].dbc, 1 << 31, true); // 允许中断

        // 设置命令
        let cmdfis = unsafe {
            ((&mut cmdtbl.cfis) as *mut [u8] as *mut usize as *mut FisRegH2D)
                .as_mut()
                .unwrap()
        };
        volatile_write!(cmdfis.fis_type, FisType::RegH2D as u8);
        volatile_set_bit!(cmdfis.pm, 1 << 7, true); // command_bit set
        volatile_write!(cmdfis.command, ATA_CMD_READ_DMA_EXT);

        volatile_write!(cmdfis.lba0, (lba_id_start & 0xFF) as u8);
        volatile_write!(cmdfis.lba1, ((lba_id_start >> 8) & 0xFF) as u8);
        volatile_write!(cmdfis.lba2, ((lba_id_start >> 16) & 0xFF) as u8);
        volatile_write!(cmdfis.lba3, ((lba_id_start >> 24) & 0xFF) as u8);
        volatile_write!(cmdfis.lba4, ((lba_id_start >> 32) & 0xFF) as u8);
        volatile_write!(cmdfis.lba5, ((lba_id_start >> 40) & 0xFF) as u8);

        volatile_write!(cmdfis.countl, (count & 0xFF) as u8);
        volatile_write!(cmdfis.counth, ((count >> 8) & 0xFF) as u8);

        volatile_write!(cmdfis.device, 1 << 6); // LBA Mode

        // 等待之前的操作完成
        let mut spin_count = 0;
        const SPIN_LIMIT: u32 = 10000;

        while (volatile_read!(port.tfd) as u8 & (ATA_DEV_BUSY | ATA_DEV_DRQ)) > 0
            && spin_count < SPIN_LIMIT
        {
            spin_count += 1;
        }

        if spin_count == SPIN_LIMIT {
            error!("Port is hung");
            return Err(SystemError::EIO);
        }

        volatile_set_bit!(port.ci, 1 << slot, true); // Issue command
                                                     // debug!("To wait ahci read complete.");
                                                     // 等待操作完成
        loop {
            if (volatile_read!(port.ci) & (1 << slot)) == 0 {
                break;
            }
            if (volatile_read!(port.is) & HBA_PxIS_TFES) > 0 {
                error!("Read disk error");
                return Err(SystemError::EIO);
            }
        }
        if let Some(kbuf) = &kbuf {
            buf.copy_from_slice(kbuf);
        }

        compiler_fence(Ordering::SeqCst);
        // successfully read
        return Ok(count * 512);
    }

    fn write_at(
        &self,
        lba_id_start: BlockId,
        count: usize,
        buf: &[u8],
    ) -> Result<usize, SystemError> {
        assert!((buf.len() & 511) == 0);
        compiler_fence(Ordering::SeqCst);
        let check_length = ((count - 1) >> 4) + 1; // prdt length
        if count * 512 > buf.len() || check_length > 8 {
            // 不可能的操作
            return Err(SystemError::E2BIG);
        } else if count == 0 {
            return Ok(0);
        }

        let port = _port(self.private().ctrl_num, self.private().port_num);

        volatile_write!(port.is, u32::MAX); // Clear pending interrupt bits

        let slot = port.find_cmdslot().unwrap_or(u32::MAX);

        if slot == u32::MAX {
            return Err(SystemError::EIO);
        }

        compiler_fence(Ordering::SeqCst);
        #[allow(unused_unsafe)]
        let cmdheader: &mut HbaCmdHeader = unsafe {
            (MMArch::phys_2_virt(PhysAddr::new(
                volatile_read!(port.clb) as usize + slot as usize * size_of::<HbaCmdHeader>(),
            ))
            .unwrap()
            .data() as *mut HbaCmdHeader)
                .as_mut()
                .unwrap()
        };
        compiler_fence(Ordering::SeqCst);

        volatile_write_bit!(
            cmdheader.cfl,
            (1 << 5) - 1_u8,
            (size_of::<FisRegH2D>() / size_of::<u32>()) as u8
        ); // Command FIS size

        volatile_set_bit!(cmdheader.cfl, 7 << 5, true); // (p,c,w)都设置为1, Read/Write bit :  Write from device
        volatile_write!(cmdheader.prdtl, check_length as u16); // PRDT entries count

        // 设置数据存放地址
        compiler_fence(Ordering::SeqCst);
        let mut buf_ptr = buf as *const [u8] as *mut usize as usize;

        // 由于目前的内存管理机制无法把用户空间的内存地址转换为物理地址，所以只能先把数据拷贝到内核空间
        // TODO：在内存管理重构后，可以直接使用用户空间的内存地址
        let user_buf = verify_area(VirtAddr::new(buf_ptr), buf.len()).is_ok();
        let mut kbuf = if user_buf {
            let mut x: Vec<u8> = vec![0; buf.len()];
            x.resize(buf.len(), 0);
            x.copy_from_slice(buf);
            Some(x)
        } else {
            None
        };

        if kbuf.is_some() {
            buf_ptr = kbuf.as_mut().unwrap().as_mut_ptr() as usize;
        }

        #[allow(unused_unsafe)]
        let cmdtbl = unsafe {
            (MMArch::phys_2_virt(PhysAddr::new(volatile_read!(cmdheader.ctba) as usize))
                .unwrap()
                .data() as *mut HbaCmdTable)
                .as_mut()
                .unwrap()
        };
        let mut tmp_count = count;
        compiler_fence(Ordering::SeqCst);

        unsafe {
            // 清空整个table的旧数据
            write_bytes(cmdtbl, 0, 1);
        }

        // 8K bytes (16 sectors) per PRDT
        for i in 0..((volatile_read!(cmdheader.prdtl) - 1) as usize) {
            volatile_write!(
                cmdtbl.prdt_entry[i].dba,
                MMArch::virt_2_phys(VirtAddr::new(buf_ptr)).unwrap().data() as u64
            );
            volatile_write_bit!(cmdtbl.prdt_entry[i].dbc, (1 << 22) - 1, 8 * 1024 - 1); // 数据长度
            volatile_set_bit!(cmdtbl.prdt_entry[i].dbc, 1 << 31, true); // 允许中断
            buf_ptr += 8 * 1024;
            tmp_count -= 16;
        }

        // Last entry
        let las = (volatile_read!(cmdheader.prdtl) - 1) as usize;
        volatile_write!(
            cmdtbl.prdt_entry[las].dba,
            MMArch::virt_2_phys(VirtAddr::new(buf_ptr)).unwrap().data() as u64
        );
        volatile_set_bit!(cmdtbl.prdt_entry[las].dbc, 1 << 31, true); // 允许中断
        volatile_write_bit!(
            cmdtbl.prdt_entry[las].dbc,
            (1 << 22) - 1,
            ((tmp_count << 9) - 1) as u32
        ); // 数据长度

        // 设置命令
        let cmdfis = unsafe {
            ((&mut cmdtbl.cfis) as *mut [u8] as *mut usize as *mut FisRegH2D)
                .as_mut()
                .unwrap()
        };
        volatile_write!(cmdfis.fis_type, FisType::RegH2D as u8);
        volatile_set_bit!(cmdfis.pm, 1 << 7, true); // command_bit set
        volatile_write!(cmdfis.command, ATA_CMD_WRITE_DMA_EXT);

        volatile_write!(cmdfis.lba0, (lba_id_start & 0xFF) as u8);
        volatile_write!(cmdfis.lba1, ((lba_id_start >> 8) & 0xFF) as u8);
        volatile_write!(cmdfis.lba2, ((lba_id_start >> 16) & 0xFF) as u8);
        volatile_write!(cmdfis.lba3, ((lba_id_start >> 24) & 0xFF) as u8);
        volatile_write!(cmdfis.lba4, ((lba_id_start >> 32) & 0xFF) as u8);
        volatile_write!(cmdfis.lba5, ((lba_id_start >> 40) & 0xFF) as u8);

        volatile_write!(cmdfis.countl, (count & 0xFF) as u8);
        volatile_write!(cmdfis.counth, ((count >> 8) & 0xFF) as u8);

        volatile_write!(cmdfis.device, 1 << 6); // LBA Mode

        volatile_set_bit!(port.ci, 1 << slot, true); // Issue command

        // 等待操作完成
        loop {
            if (volatile_read!(port.ci) & (1 << slot)) == 0 {
                break;
            }
            if (volatile_read!(port.is) & HBA_PxIS_TFES) > 0 {
                error!("Write disk error");
                return Err(SystemError::EIO);
            }
        }

        compiler_fence(Ordering::SeqCst);
        // successfully read
        return Ok(count * 512);
    }

    fn sync(&self) -> Result<(), SystemError> {
        // 由于目前没有block cache, 因此sync返回成功即可
        return Ok(());
    }
}

impl KObject for AhciDisk {
    fn as_any_ref(&self) -> &dyn core::any::Any {
        self
    }

    fn inode(&self) -> Option<Arc<KernFSInode>> {
        self.inner().kobject_common.kern_inode.clone()
    }

    fn kobj_type(&self) -> Option<&'static dyn KObjType> {
        self.inner().kobject_common.kobj_type
    }

    fn kset(&self) -> Option<Arc<KSet>> {
        self.inner().kobject_common.kset.clone()
    }

    fn parent(&self) -> Option<Weak<dyn KObject>> {
        self.inner().kobject_common.parent.clone()
    }

    fn set_inode(&self, inode: Option<Arc<KernFSInode>>) {
        self.inner().kobject_common.kern_inode = inode;
    }

    fn kobj_state(&self) -> RwLockReadGuard<KObjectState> {
        self.locked_kobject_state.read()
    }

    fn kobj_state_mut(&self) -> RwLockWriteGuard<KObjectState> {
        self.locked_kobject_state.write()
    }

    fn set_kobj_state(&self, state: KObjectState) {
        *self.locked_kobject_state.write() = state;
    }

    fn name(&self) -> alloc::string::String {
        todo!()
    }

    fn set_name(&self, _name: alloc::string::String) {
        todo!()
    }

    fn set_kset(&self, kset: Option<Arc<KSet>>) {
        self.inner().kobject_common.kset = kset
    }

    fn set_parent(&self, parent: Option<Weak<dyn KObject>>) {
        self.inner().kobject_common.parent = parent
    }

    fn set_kobj_type(&self, ktype: Option<&'static dyn KObjType>) {
        self.inner().kobject_common.kobj_type = ktype
    }
}

impl Device for AhciDisk {
    fn dev_type(&self) -> DeviceType {
        return DeviceType::Block;
    }

    fn id_table(&self) -> IdTable {
        IdTable::new(AHCI_DISK_BASENAME.to_string(), None)
    }

    fn bus(&self) -> Option<Weak<dyn Bus>> {
        self.inner().device_common.bus.clone()
    }

    fn set_bus(&self, bus: Option<Weak<dyn Bus>>) {
        self.inner().device_common.bus = bus;
    }

    fn driver(&self) -> Option<Arc<dyn Driver>> {
        let r = self.inner().device_common.driver.clone()?.upgrade();
        if r.is_none() {
            self.inner().device_common.driver = None;
        }

        return r;
    }

    fn is_dead(&self) -> bool {
        false
    }

    fn set_driver(&self, driver: Option<Weak<dyn Driver>>) {
        self.inner().device_common.driver = driver;
    }

    fn can_match(&self) -> bool {
        self.inner().device_common.can_match
    }

    fn set_can_match(&self, can_match: bool) {
        self.inner().device_common.can_match = can_match;
    }

    fn state_synced(&self) -> bool {
        true
    }

    fn set_class(&self, class: Option<Weak<dyn Class>>) {
        self.inner().device_common.class = class;
    }
}

impl BlockDevice for AhciDisk {
    #[inline]
    fn as_any_ref(&self) -> &dyn core::any::Any {
        self
    }

    #[inline]
    fn blk_size_log2(&self) -> u8 {
        9
    }

    fn sync(&self) -> Result<(), SystemError> {
        return self.sync();
    }

    #[inline]
    fn device(&self) -> Arc<dyn Device> {
        return self.self_ref.upgrade().unwrap();
    }

    fn block_size(&self) -> usize {
        todo!()
    }

    fn partitions(&self) -> Vec<Arc<Partition>> {
        return self.private().partitions.clone();
    }

    #[inline]
    fn read_at_sync(
        &self,
        lba_id_start: BlockId, // 起始lba编号
        count: usize,          // 读取lba的数量
        buf: &mut [u8],
    ) -> Result<usize, SystemError> {
        self.read_at(lba_id_start, count, buf)
    }

    #[inline]
    fn write_at_sync(
        &self,
        lba_id_start: BlockId,
        count: usize,
        buf: &[u8],
    ) -> Result<usize, SystemError> {
        self.write_at(lba_id_start, count, buf)
    }
}

#[derive(Debug)]
#[cast_to([sync] Driver)]
struct AhciDiskDriver {
    inner: SpinLock<InnerAhciDiskDriver>,
    kobject_state: LockedKObjectState,
}

#[derive(Debug)]
struct InnerAhciDiskDriver {
    driver_common_data: DriverCommonData,
    kobject_common_data: KObjectCommonData,
}

impl AhciDiskDriver {
    fn new() -> Arc<Self> {
        let inner = InnerAhciDiskDriver {
            driver_common_data: DriverCommonData::default(),
            kobject_common_data: KObjectCommonData::default(),
        };
        Arc::new(AhciDiskDriver {
            inner: SpinLock::new(inner),
            kobject_state: LockedKObjectState::default(),
        })
    }

    fn inner(&self) -> SpinLockGuard<InnerAhciDiskDriver> {
        self.inner.lock()
    }
}

impl Driver for AhciDiskDriver {
    fn coredump(&self, _device: &Arc<dyn Device>) -> Result<(), SystemError> {
        todo!()
    }

    fn id_table(&self) -> Option<IdTable> {
        Some(IdTable::new(AHCI_DISK_BASENAME.to_string(), None))
    }

    fn devices(&self) -> Vec<Arc<dyn Device>> {
        self.inner().driver_common_data.devices.clone()
    }

    fn add_device(&self, device: Arc<dyn Device>) {
        let iface = device
            .arc_any()
            .downcast::<AhciDisk>()
            .expect("AhciDiskDriver::add_device() failed: device is not a AhciDisk");
        self.inner()
            .driver_common_data
            .devices
            .push(iface as Arc<dyn Device>)
    }

    fn delete_device(&self, device: &Arc<dyn Device>) {
        let _iface = device
            .clone()
            .arc_any()
            .downcast::<AhciDisk>()
            .expect("AhciDiskDriver::delete_device() failed: device is not a AhciDisk");
        let mut guard = self.inner();
        let index = guard
            .driver_common_data
            .devices
            .iter()
            .position(|dev| Arc::ptr_eq(device, dev))
            .expect("AhciDiskDriver::delete_device() failed: device not found");
    }

    fn __find_device_by_name_fast(&self, _name: &str) -> Option<Arc<dyn Device>> {
        todo!()
    }

    fn suppress_bind_attrs(&self) -> bool {
        todo!()
    }

    fn bus(&self) -> Option<Weak<dyn Bus>> {
        self.inner().driver_common_data.bus.clone()
    }

    fn set_bus(&self, bus: Option<Weak<dyn Bus>>) {
        self.inner().driver_common_data.bus = bus;
    }

    fn groups(&self) -> &'static [&'static dyn AttributeGroup] {
        todo!()
    }

    fn dev_groups(&self) -> &'static [&'static dyn AttributeGroup] {
        todo!()
    }

    fn probe_type(&self) -> DriverProbeType {
        todo!()
    }
}

impl KObject for AhciDiskDriver {
    fn as_any_ref(&self) -> &dyn Any {
        self
    }

    fn set_inode(&self, inode: Option<Arc<KernFSInode>>) {
        self.inner().kobject_common_data.kern_inode = inode
    }

    fn inode(&self) -> Option<Arc<KernFSInode>> {
        self.inner().kobject_common_data.kern_inode.clone()
    }

    fn parent(&self) -> Option<Weak<dyn KObject>> {
        self.inner().kobject_common_data.parent.clone()
    }

    fn set_parent(&self, parent: Option<Weak<dyn KObject>>) {
        self.inner().kobject_common_data.parent = parent
    }

    fn kset(&self) -> Option<Arc<KSet>> {
        self.inner().kobject_common_data.kset.clone()
    }

    fn set_kset(&self, kset: Option<Arc<KSet>>) {
        self.inner().kobject_common_data.kset = kset
    }

    fn kobj_type(&self) -> Option<&'static dyn KObjType> {
        self.inner().kobject_common_data.kobj_type
    }

    fn set_kobj_type(&self, ktype: Option<&'static dyn KObjType>) {
        self.inner().kobject_common_data.kobj_type = ktype
    }

    fn name(&self) -> String {
        AHCI_DISK_BASENAME.to_string()
    }

    fn set_name(&self, _name: String) {}

    fn kobj_state(&self) -> RwLockReadGuard<KObjectState> {
        self.kobject_state.read()
    }

    fn kobj_state_mut(&self) -> RwLockWriteGuard<KObjectState> {
        self.kobject_state.write()
    }

    fn set_kobj_state(&self, state: KObjectState) {
        *self.kobject_state.write() = state
    }
}
