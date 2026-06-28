//! Device-level Vulkan state: library, instance, physical device, logical device,
//! queue, and the standard allocators. Created once in `App::new` and shared (via
//! `Arc`) across window/swapchain recreation.

use std::sync::Arc;

use anyhow::{Context, anyhow};
use vulkano::Version;
use vulkano::VulkanLibrary;
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::device::physical::{PhysicalDevice, PhysicalDeviceType};
use vulkano::device::{
    Device, DeviceCreateInfo, DeviceExtensions, DeviceFeatures, Queue, QueueCreateInfo, QueueFlags,
};
use vulkano::instance::{Instance, InstanceCreateFlags, InstanceCreateInfo};
use vulkano::memory::allocator::StandardMemoryAllocator;
use vulkano::swapchain::Surface;
use winit::event_loop::EventLoop;

/// Device-level Vulkan handles. Cheap to clone the inner `Arc`s; survives resize.
pub struct VulkanContext {
    pub instance: Arc<Instance>,
    pub physical_device: Arc<PhysicalDevice>,
    pub device: Arc<Device>,
    pub queue: Arc<Queue>,
    // Allocated now as standard device-level state; unused until Step 1 grows
    // vertex/uniform buffers and descriptor sets.
    #[allow(dead_code)]
    pub memory_allocator: Arc<StandardMemoryAllocator>,
    pub command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    #[allow(dead_code)]
    pub descriptor_set_allocator: Arc<StandardDescriptorSetAllocator>,
}

impl VulkanContext {
    /// Initialize Vulkan from the event loop (needed to query the surface extensions
    /// required for the current windowing system).
    pub fn new(event_loop: &EventLoop<()>) -> anyhow::Result<Self> {
        let library = VulkanLibrary::new().context("loading the Vulkan library")?;

        let instance = Instance::new(
            library,
            InstanceCreateInfo {
                flags: InstanceCreateFlags::ENUMERATE_PORTABILITY,
                enabled_extensions: Surface::required_extensions(event_loop)
                    .context("querying required surface extensions")?,
                ..Default::default()
            },
        )
        .context("creating the Vulkan instance")?;

        // We draw with dynamic rendering (Vulkan 1.3 core, or the KHR extension on
        // older devices) and present via the swapchain extension.
        let (physical_device, queue_family_index) = Self::select_physical_device(&instance)?;

        let mut device_extensions = DeviceExtensions {
            khr_swapchain: true,
            ..DeviceExtensions::empty()
        };
        // Pre-1.3 devices expose dynamic rendering only via the extension.
        if physical_device.api_version() < Version::V1_3 {
            device_extensions.khr_dynamic_rendering = true;
        }

        let (device, mut queues) = Device::new(
            physical_device.clone(),
            DeviceCreateInfo {
                enabled_extensions: device_extensions,
                enabled_features: DeviceFeatures {
                    dynamic_rendering: true,
                    ..DeviceFeatures::empty()
                },
                queue_create_infos: vec![QueueCreateInfo {
                    queue_family_index,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .context("creating the logical device")?;

        let queue = queues
            .next()
            .ok_or_else(|| anyhow!("device created with no queues"))?;

        let memory_allocator = Arc::new(StandardMemoryAllocator::new_default(device.clone()));
        let command_buffer_allocator = Arc::new(StandardCommandBufferAllocator::new(
            device.clone(),
            Default::default(),
        ));
        let descriptor_set_allocator = Arc::new(StandardDescriptorSetAllocator::new(
            device.clone(),
            Default::default(),
        ));

        log::info!(
            "Vulkan ready: {} (type {:?}, Vulkan {})",
            physical_device.properties().device_name,
            physical_device.properties().device_type,
            physical_device.api_version(),
        );

        Ok(Self {
            instance,
            physical_device,
            device,
            queue,
            memory_allocator,
            command_buffer_allocator,
            descriptor_set_allocator,
        })
    }

    /// Pick a graphics-capable device supporting dynamic rendering + swapchain,
    /// preferring discrete over integrated GPUs. Present support is verified later
    /// against the actual surface (on SteamOS the graphics family also presents).
    fn select_physical_device(
        instance: &Arc<Instance>,
    ) -> anyhow::Result<(Arc<PhysicalDevice>, u32)> {
        instance
            .enumerate_physical_devices()
            .context("enumerating physical devices")?
            .filter(|p| {
                p.api_version() >= Version::V1_3 || p.supported_extensions().khr_dynamic_rendering
            })
            .filter(|p| p.supported_extensions().khr_swapchain)
            .filter_map(|p| {
                p.queue_family_properties()
                    .iter()
                    .position(|q| q.queue_flags.intersects(QueueFlags::GRAPHICS))
                    .map(|i| (p, i as u32))
            })
            .min_by_key(|(p, _)| match p.properties().device_type {
                PhysicalDeviceType::DiscreteGpu => 0,
                PhysicalDeviceType::IntegratedGpu => 1,
                PhysicalDeviceType::VirtualGpu => 2,
                PhysicalDeviceType::Cpu => 3,
                _ => 4,
            })
            .ok_or_else(|| {
                anyhow!(
                    "no suitable Vulkan device found (need graphics + swapchain + dynamic \
                     rendering); sanity-check the GPU/loader with `vulkaninfo | head`"
                )
            })
    }
}
