use crate::{
    drivers::{BLOCK_DEVICE, UART_DEVICE, PLIC_DRIVE},
    sync::{SpinMutex, UPSafeCell},
    task::{add_task, schedule, take_current_task, TaskContext, TaskControlBlock, TaskStatus, current_hartid},
};
use alloc::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};
use k210_pac::Interrupt;
use k210_soc::{
    dmac::channel_interrupt_clear,
    plic::{clear_irq, current_irq, plic_enable, set_priority, set_thershold},
    sysctl::dma_channel,
};
use lazy_static::lazy_static;
use riscv::register::{sie, sstatus};

lazy_static! {
    pub static ref FLAG: UPSafeCell<bool> = unsafe { UPSafeCell::new(true) };
    pub static ref IRQMANAGER: Arc<SpinMutex<IrqManager>> =
        Arc::new(SpinMutex::new(IrqManager::new()));
}

pub struct IrqManager {
    plic_instance: BTreeMap<usize, VecDeque<Arc<TaskControlBlock>>>,
}

impl IrqManager {
    pub fn new() -> Self {
        let plic_instance = BTreeMap::new();
        Self { plic_instance }
    }

    pub fn register_irq(&mut self, source: Interrupt) {
        PLIC_DRIVE.enable(source, current_hartid());
        PLIC_DRIVE.set_priority(1, source);
        self.plic_instance.insert(source as usize, VecDeque::new());
    }
    pub fn inqueue(&mut self, irq: usize, task: Arc<TaskControlBlock>) {
        if let Some(queue) = self.plic_instance.get_mut(&irq) {
            queue.push_back(task)
        }
    }
    pub fn dequeue(&mut self, irq: usize) -> Option<Arc<TaskControlBlock>> {
        if let Some(queue) = self.plic_instance.get_mut(&irq) {
            queue.pop_front()
        } else {
            None
        }
    }
}

pub fn irq_init(hartid: usize) {
    unsafe {
        sie::set_ssoft();
        PLIC_DRIVE.set_thershold(0, hartid);
        let mut irq_manager = IRQMANAGER.lock();
        irq_manager.register_irq(Interrupt::DMA0);
        irq_manager.register_irq(Interrupt::UARTHS);
        println!("Interrupt Init Ok");
    }
}

#[no_mangle]
pub fn wait_for_irq_and_run_next(irq: usize) {
    if let Some(task) = take_current_task() {
        let mut task_inner = task.inner_lock_access();
        task_inner.task_status = TaskStatus::Waiting;
        let task_cx_ptr = &mut task_inner.task_cx as *mut TaskContext;
        drop(task_inner);
        IRQMANAGER.lock().inqueue(irq, task);
        schedule(task_cx_ptr);
    } else {
        panic!("too eaily irq")
    }
}

pub fn handler_ext() {
    let mut irq_manager = IRQMANAGER.lock();
    let irq = PLIC_DRIVE.current(current_hartid());
    match irq {
        27 => {
            BLOCK_DEVICE.handler_interrupt();
            let task = irq_manager.dequeue(irq).unwrap();
            add_task(task);
        }
        33 => {
            UART_DEVICE.handler_interrupt();
            match irq_manager.dequeue(irq) {
                Some(task) => add_task(task),
                None => {}
            }
        }
        _ => {
            panic!("unknow irq")
        }
    }
    PLIC_DRIVE.clear(irq, current_hartid());
}
