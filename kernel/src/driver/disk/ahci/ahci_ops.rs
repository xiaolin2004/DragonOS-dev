fn ahci_set_lpm(link: &mut AtaLink, policy: AtaLpmPolicy, hints: u32) -> i32 {
    // TODO: Implement this function
}

fn ahci_led_show(ap: &mut AtaPort, buf: &mut str) -> isize {
    // TODO: Implement this function
}

fn ahci_led_store(ap: &mut AtaPort, buf: &str, size: usize) -> isize {
    // TODO: Implement this function
}

fn ahci_transmit_led_message(ap: &mut AtaPort, state: u32, size: isize) -> isize {
    // TODO: Implement this function
}

fn ahci_scr_read(link: &mut AtaLink, sc_reg: u32, val: &mut u32) -> i32 {
    // TODO: Implement this function
}

fn ahci_scr_write(link: &mut AtaLink, sc_reg: u32, val: u32) -> i32 {
    // TODO: Implement this function
}

fn ahci_qc_fill_rtf(qc: &mut AtaQueuedCmd) {
    // TODO: Implement this function
}

fn ahci_qc_ncq_fill_rtf(ap: &mut AtaPort, done_mask: u64) {
    // TODO: Implement this function
}

fn ahci_port_start(ap: &mut AtaPort) -> i32 {
    // TODO: Implement this function
}

fn ahci_port_stop(ap: &mut AtaPort) {
    // TODO: Implement this function
}

fn ahci_qc_prep(qc: &mut AtaQueuedCmd) -> AtaCompletionErrors {
    // TODO: Implement this function
}

fn ahci_pmp_qc_defer(qc: &mut AtaQueuedCmd) -> i32 {
    // TODO: Implement this function
}

fn ahci_freeze(ap: &mut AtaPort) {
    // TODO: Implement this function
}

fn ahci_thaw(ap: &mut AtaPort) {
    // TODO: Implement this function
}

fn ahci_set_aggressive_devslp(ap: &mut AtaPort, sleep: bool) {
    // TODO: Implement this function
}

fn ahci_enable_fbs(ap: &mut AtaPort) {
    // TODO: Implement this function
}

fn ahci_disable_fbs(ap: &mut AtaPort) {
    // TODO: Implement this function
}

fn ahci_pmp_attach(ap: &mut AtaPort) {
    // TODO: Implement this function
}

fn ahci_pmp_detach(ap: &mut AtaPort) {
    // TODO: Implement this function
}

fn ahci_softreset(link: &mut AtaLink, class: &mut u32, deadline: u64) -> i32 {
    // TODO: Implement this function
}

fn ahci_pmp_retry_softreset(link: &mut AtaLink, class: &mut u32, deadline: u64) -> i32 {
    // TODO: Implement this function
}

fn ahci_hardreset(link: &mut AtaLink, class: &mut u32, deadline: u64) -> i32 {
    // TODO: Implement this function
}

fn ahci_postreset(link: &mut AtaLink, class: &mut u32) {
    // TODO: Implement this function
}

fn ahci_post_internal_cmd(qc: &mut AtaQueuedCmd) {
    // TODO: Implement this function
}

fn ahci_dev_config(dev: &mut AtaDevice) {
    // TODO: Implement this function
}

fn ahci_port_suspend(ap: &mut AtaPort, mesg: PmMessage) -> i32 {
    // TODO: Implement this function
}

fn ahci_activity_show(dev: &mut AtaDevice, buf: &mut str) -> isize {
    // TODO: Implement this function
}

fn ahci_activity_store(dev: &mut AtaDevice, val: SwActivity) -> isize {
    // TODO: Implement this function
}

fn ahci_init_sw_activity(link: &mut AtaLink) {
    // TODO: Implement this function
}

fn ahci_show_host_caps(dev: &mut Device, attr: &mut DeviceAttribute, buf: &mut str) -> isize {
    // TODO: Implement this function
}

fn ahci_show_host_cap2(dev: &mut Device, attr: &mut DeviceAttribute, buf: &mut str) -> isize {
    // TODO: Implement this function
}

fn ahci_show_host_version(dev: &mut Device, attr: &mut DeviceAttribute, buf: &mut str) -> isize {
    // TODO: Implement this function
}

fn ahci_show_port_cmd(dev: &mut Device, attr: &mut DeviceAttribute, buf: &mut str) -> isize {
    // TODO: Implement this function
}

fn ahci_read_em_buffer(dev: &mut Device, attr: &mut DeviceAttribute, buf: &mut str) -> isize {
    // TODO: Implement this function
}

fn ahci_store_em_buffer(dev: &mut Device, attr: &mut DeviceAttribute, buf: &str, size: usize) -> isize {
    // TODO: Implement this function
}

fn ahci_show_em_supported(dev: &mut Device, attr: &mut DeviceAttribute, buf: &mut str) -> isize {
    // TODO: Implement this function
}

fn ahci_single_level_irq_intr(irq: i32, dev_instance: &mut ()) -> Irqreturn {
    // TODO: Implement this function
}
