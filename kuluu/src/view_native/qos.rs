use bevy::prelude::*;

// Hybrid-core scheduling is per-OS: macOS QoS classes steer P- vs E-cores
// directly; Windows steers via thread priority (plus EcoQoS power throttling,
// see kuluu-3q8t for the follow-up); Linux only offers per-thread niceness and
// leaves core placement to capacity-aware scheduling. No OS exposes pinning we
// should use, so this module classifies threads by ROLE and translates per-OS.
// Observed motivation: frame-critical threads at default class get parked on
// E-cores under contention (E-cores pegged, P-cores idle in a Jeuno crowd).
#[derive(Clone, Copy)]
pub enum ThreadClass {
    FrameCritical,
    AsyncCompute,
    Background,
}

#[cfg(target_os = "macos")]
mod imp {
    use super::ThreadClass;

    // XNU pthread/qos.h class values.
    const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;
    const QOS_CLASS_USER_INITIATED: u32 = 0x19;
    const QOS_CLASS_UTILITY: u32 = 0x11;

    extern "C" {
        fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
    }

    pub fn classify_current_thread(class: ThreadClass) {
        let qos = match class {
            ThreadClass::FrameCritical => QOS_CLASS_USER_INTERACTIVE,
            ThreadClass::AsyncCompute => QOS_CLASS_USER_INITIATED,
            ThreadClass::Background => QOS_CLASS_UTILITY,
        };
        // SAFETY: *_self_np only touches the calling thread's scheduling class;
        // no pointers cross the boundary and invalid classes are rejected
        // wholesale via the return code.
        let rc = unsafe { pthread_set_qos_class_self_np(qos, 0) };
        if rc != 0 {
            bevy::log::warn!("pthread_set_qos_class_self_np({qos:#x}) failed: {rc}");
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use super::ThreadClass;

    // processthreadsapi.h values; ABOVE_NORMAL for frame threads is the
    // conventional game setting — Windows' scheduler and Intel Thread Director
    // steer by priority/utilization, and EcoQoS opt-in for Background threads
    // is a follow-up needing real hybrid-Windows measurement (kuluu-3q8t).
    const THREAD_PRIORITY_ABOVE_NORMAL: i32 = 1;
    const THREAD_PRIORITY_BELOW_NORMAL: i32 = -1;

    extern "system" {
        fn GetCurrentThread() -> isize;
        fn SetThreadPriority(thread: isize, priority: i32) -> i32;
    }

    pub fn classify_current_thread(class: ThreadClass) {
        let priority = match class {
            ThreadClass::FrameCritical => THREAD_PRIORITY_ABOVE_NORMAL,
            ThreadClass::AsyncCompute => return,
            ThreadClass::Background => THREAD_PRIORITY_BELOW_NORMAL,
        };
        // SAFETY: GetCurrentThread returns a pseudo-handle needing no close;
        // SetThreadPriority on it affects only the calling thread.
        let ok = unsafe { SetThreadPriority(GetCurrentThread(), priority) };
        if ok == 0 {
            bevy::log::warn!("SetThreadPriority({priority}) failed");
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::ThreadClass;

    // Only Background is demoted: raising priority above default needs
    // privileges on Linux, and default niceness already competes fine.
    const BACKGROUND_NICE: i32 = 5;
    const PRIO_PROCESS: i32 = 0;

    extern "C" {
        fn setpriority(which: i32, who: u32, prio: i32) -> i32;
        fn gettid() -> i32;
    }

    pub fn classify_current_thread(class: ThreadClass) {
        if !matches!(class, ThreadClass::Background) {
            return;
        }
        // SAFETY: setpriority with the calling thread's tid affects only this
        // thread's niceness; failure is reported via the return code.
        unsafe { setpriority(PRIO_PROCESS, gettid() as u32, BACKGROUND_NICE) };
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod imp {
    pub fn classify_current_thread(_class: super::ThreadClass) {}
}

pub use imp::classify_current_thread;

// Mirrors bevy_app task_pool_plugin.rs TaskPoolOptions defaults (io 25% clamp
// 1..=4 of total, async-compute likewise from the remainder, compute = rest)
// so pool sizes match what TaskPoolPlugin would have built; get_or_init before
// DefaultPlugins makes these pools win and create_default_pools no-op.
fn default_partition(total: usize) -> (usize, usize, usize) {
    // clamp AFTER min(remaining): bevy's formula lets min-floors oversubscribe
    // tiny machines rather than starve a pool.
    let slice = |remaining: usize| ((total as f32 * 0.25) as usize).min(remaining).clamp(1, 4);
    let io = slice(total);
    let async_compute = slice(total.saturating_sub(io));
    let compute = total.saturating_sub(io + async_compute).max(1);
    (io, async_compute, compute)
}

pub fn init_task_pools_with_qos() {
    use bevy::tasks::{AsyncComputeTaskPool, ComputeTaskPool, IoTaskPool, TaskPoolBuilder};
    let total = std::thread::available_parallelism().map_or(1, |n| n.get());
    let (io, async_compute, compute) = default_partition(total);

    IoTaskPool::get_or_init(|| {
        TaskPoolBuilder::new()
            .num_threads(io)
            .thread_name("IO Task Pool".to_string())
            .on_thread_spawn(|| classify_current_thread(ThreadClass::Background))
            .build()
    });
    AsyncComputeTaskPool::get_or_init(|| {
        TaskPoolBuilder::new()
            .num_threads(async_compute)
            .thread_name("Async Compute Taskpool".to_string())
            .on_thread_spawn(|| classify_current_thread(ThreadClass::AsyncCompute))
            .build()
    });
    ComputeTaskPool::get_or_init(|| {
        TaskPoolBuilder::new()
            .num_threads(compute)
            .thread_name("Compute Task Pool".to_string())
            .on_thread_spawn(|| classify_current_thread(ThreadClass::FrameCritical))
            .build()
    });
}

// Runs in the Render schedule because that is the only place guaranteed to
// execute on the pipelined render thread (RenderStartup can run on the main
// thread before the sub-app migrates).
pub fn promote_render_thread(mut promoted: Local<bool>) {
    if *promoted {
        return;
    }
    *promoted = true;
    classify_current_thread(ThreadClass::FrameCritical);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_invariants_hold_across_core_counts() {
        for total in 1..=128 {
            let (io, ac, compute) = default_partition(total);
            assert!((1..=4).contains(&io), "io {io} out of range at {total}");
            assert!((1..=4).contains(&ac), "async {ac} out of range at {total}");
            assert!(compute >= 1, "compute floor at {total}");
            assert!(
                io + ac + compute <= total.max(3),
                "oversubscribed beyond min-floors at {total}"
            );
        }
    }

    #[test]
    fn partition_mirrors_bevy_default_examples() {
        // Spot values computed from bevy_app's formula (25% trunc, clamp 1..=4,
        // sequential remainder), not from any particular machine.
        assert_eq!(default_partition(4), (1, 1, 2));
        assert_eq!(default_partition(8), (2, 2, 4));
        assert_eq!(default_partition(16), (4, 4, 8));
        assert_eq!(default_partition(32), (4, 4, 24));
    }

    #[test]
    fn classify_runs_without_error_on_a_real_thread() {
        std::thread::spawn(|| {
            classify_current_thread(ThreadClass::Background);
            classify_current_thread(ThreadClass::FrameCritical);
        })
        .join()
        .unwrap();
    }
}
