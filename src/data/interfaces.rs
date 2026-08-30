// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Edgehog interfaces to files to embed into the program.

use astarte_device_sdk::builder::DeviceBuilder;

const BASE_IMAGE: &str =
    include_str!("../../deps/interfaces/io.edgehog.devicemanager.BaseImage.json");
const BATTERY_STATUS: &str =
    include_str!("../../deps/interfaces/io.edgehog.devicemanager.BatteryStatus.json");
const COMMANDS: &str = include_str!("../../deps/interfaces/io.edgehog.devicemanager.Commands.json");
const HARDWARE_INFO: &str =
    include_str!("../../deps/interfaces/io.edgehog.devicemanager.HardwareInfo.json");
const OS_INFO: &str = include_str!("../../deps/interfaces/io.edgehog.devicemanager.OSInfo.json");
const RUNTIME_INFO: &str =
    include_str!("../../deps/interfaces/io.edgehog.devicemanager.RuntimeInfo.json");
const STORAGE_USAGE: &str =
    include_str!("../../deps/interfaces/io.edgehog.devicemanager.StorageUsage.json");
const SYSTEM_INFO: &str =
    include_str!("../../deps/interfaces/io.edgehog.devicemanager.SystemInfo.json");
const SYSTEM_STATUS: &str =
    include_str!("../../deps/interfaces/io.edgehog.devicemanager.SystemStatus.json");
const CONFIG_TELEMETRY: &str =
    include_str!("../../deps/interfaces/io.edgehog.devicemanager.config.Telemetry.json");

/// Add the enabled interfaces
pub(crate) fn add_interfaces<C, S>(
    mut builder: DeviceBuilder<C, S>,
) -> eyre::Result<DeviceBuilder<C, S>> {
    builder = builder
        .interface_str(BASE_IMAGE)?
        .interface_str(BATTERY_STATUS)?
        .interface_str(COMMANDS)?
        .interface_str(HARDWARE_INFO)?
        .interface_str(OS_INFO)?
        .interface_str(RUNTIME_INFO)?
        .interface_str(STORAGE_USAGE)?
        .interface_str(SYSTEM_INFO)?
        .interface_str(SYSTEM_STATUS)?
        .interface_str(CONFIG_TELEMETRY)?;

    #[cfg(all(feature = "zbus", target_os = "linux"))]
    {
        const CELLULAR_CONNECTION_PROPERTIES: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.CellularConnectionProperties.json"
        );
        const GEOLOCATION: &str =
            include_str!("../../deps/interfaces/io.edgehog.devicemanager.Geolocation.json");
        const CELLULAR_CONNECTION_STATUS: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.CellularConnectionStatus.json"
        );
        const LED_BEHAVIOR: &str =
            include_str!("../../deps/interfaces/io.edgehog.devicemanager.LedBehavior.json");
        const OTA_EVENT: &str =
            include_str!("../../deps/interfaces/io.edgehog.devicemanager.OTAEvent.json");
        const OTA_REQUEST: &str =
            include_str!("../../deps/interfaces/io.edgehog.devicemanager.OTARequest.json");

        builder = builder
            .interface_str(CELLULAR_CONNECTION_PROPERTIES)?
            .interface_str(GEOLOCATION)?
            .interface_str(CELLULAR_CONNECTION_STATUS)?
            .interface_str(LED_BEHAVIOR)?
            .interface_str(OTA_EVENT)?
            .interface_str(OTA_REQUEST)?;
    }

    #[cfg(feature = "forwarder")]
    {
        const FORWARDER_SESSION_REQUEST: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.ForwarderSessionRequest.json"
        );
        const FORWARDER_SESSION_STATE: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.ForwarderSessionState.json"
        );

        builder = builder
            .interface_str(FORWARDER_SESSION_REQUEST)?
            .interface_str(FORWARDER_SESSION_STATE)?;
    }

    #[cfg(all(feature = "udev", target_os = "linux"))]
    {
        const NETWORK_INTERFACE_PROPERTIES: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.NetworkInterfaceProperties.json"
        );

        builder = builder.interface_str(NETWORK_INTERFACE_PROPERTIES)?;
    }

    #[cfg(feature = "wifiscanner")]
    {
        const WIFI_SCAN_RESULTS: &str =
            include_str!("../../deps/interfaces/io.edgehog.devicemanager.WiFiScanResults.json");

        builder = builder.interface_str(WIFI_SCAN_RESULTS)?;
    }

    #[cfg(feature = "containers")]
    {
        const APPS_AVAILABLE_CONTAINERS: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.AvailableContainers.json"
        );
        const APPS_AVAILABLE_DEPLOYMENTS: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.AvailableDeployments.json"
        );
        const APPS_AVAILABLE_DEVICE_MAPPINGS: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.AvailableDeviceMappings.json"
        );
        const APPS_AVAILABLE_DEVICE_REQUESTS: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.AvailableDeviceRequests.json"
        );
        const APPS_AVAILABLE_IMAGES: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.AvailableImages.json"
        );
        const APPS_AVAILABLE_NETWORKS: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.AvailableNetworks.json"
        );
        const APPS_AVAILABLE_VOLUMES: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.AvailableVolumes.json"
        );
        const APPS_CREATE_CONTAINER_REQUEST: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.CreateContainerRequest.json"
        );
        const APPS_CREATE_DEPLOYMENT_REQUEST: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.CreateDeploymentRequest.json"
        );
        const APPS_CREATE_DEVICE_MAPPING_REQUEST: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.CreateDeviceMappingRequest.json"
        );
        const APPS_CREATE_DEVICE_REQUEST: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.CreateDeviceRequest.json"
        );
        const APPS_CREATE_IMAGE_REQUEST: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.CreateImageRequest.json"
        );
        const APPS_CREATE_NETWORK_REQUEST: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.CreateNetworkRequest.json"
        );
        const APPS_CREATE_VOLUME_REQUEST: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.CreateVolumeRequest.json"
        );
        const APPS_DEPLOYMENT_COMMAND: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.DeploymentCommand.json"
        );
        const APPS_DEPLOYMENT_EVENT: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.DeploymentEvent.json"
        );
        const APPS_DEPLOYMENT_UPDATE: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.DeploymentUpdate.json"
        );
        const APPS_STATS_CONTAINER_BLKIO: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.stats.ContainerBlkio.json"
        );
        const APPS_STATS_CONTAINER_CPU: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.stats.ContainerCpu.json"
        );
        const APPS_STATS_CONTAINER_MEMORY: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.stats.ContainerMemory.json"
        );
        const APPS_STATS_CONTAINER_MEMORY_STATS: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.stats.ContainerMemoryStats.json"
        );
        const APPS_STATS_CONTAINER_NETWORKS: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.stats.ContainerNetworks.json"
        );
        const APPS_STATS_CONTAINER_PROCESSES: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.stats.ContainerProcesses.json"
        );
        const APPS_STATS_VOLUME_USAGE: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.apps.stats.VolumeUsage.json"
        );

        builder = builder
            .interface_str(APPS_AVAILABLE_CONTAINERS)?
            .interface_str(APPS_AVAILABLE_DEPLOYMENTS)?
            .interface_str(APPS_AVAILABLE_DEVICE_MAPPINGS)?
            .interface_str(APPS_AVAILABLE_DEVICE_REQUESTS)?
            .interface_str(APPS_AVAILABLE_IMAGES)?
            .interface_str(APPS_AVAILABLE_NETWORKS)?
            .interface_str(APPS_AVAILABLE_VOLUMES)?
            .interface_str(APPS_CREATE_CONTAINER_REQUEST)?
            .interface_str(APPS_CREATE_DEPLOYMENT_REQUEST)?
            .interface_str(APPS_CREATE_DEVICE_MAPPING_REQUEST)?
            .interface_str(APPS_CREATE_DEVICE_REQUEST)?
            .interface_str(APPS_CREATE_IMAGE_REQUEST)?
            .interface_str(APPS_CREATE_NETWORK_REQUEST)?
            .interface_str(APPS_CREATE_VOLUME_REQUEST)?
            .interface_str(APPS_DEPLOYMENT_COMMAND)?
            .interface_str(APPS_DEPLOYMENT_EVENT)?
            .interface_str(APPS_DEPLOYMENT_UPDATE)?
            .interface_str(APPS_STATS_CONTAINER_BLKIO)?
            .interface_str(APPS_STATS_CONTAINER_CPU)?
            .interface_str(APPS_STATS_CONTAINER_MEMORY)?
            .interface_str(APPS_STATS_CONTAINER_MEMORY_STATS)?
            .interface_str(APPS_STATS_CONTAINER_NETWORKS)?
            .interface_str(APPS_STATS_CONTAINER_PROCESSES)?
            .interface_str(APPS_STATS_VOLUME_USAGE)?;
    }

    #[cfg(feature = "file-transfer")]
    {
        const FILE_TRANSFER_CAPABILITIES: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.fileTransfer.Capabilities.json"
        );
        const FILE_TRANSFER_DEVICE_TO_SERVER: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.fileTransfer.DeviceToServer.json"
        );
        const FILE_TRANSFER_PROGRESS: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.fileTransfer.Progress.json"
        );
        const FILE_TRANSFER_RESPONSE: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.fileTransfer.Response.json"
        );
        const FILE_TRANSFER_SERVER_TO_DEVICE: &str = include_str!(
            "../../deps/interfaces/io.edgehog.devicemanager.fileTransfer.ServerToDevice.json"
        );
        const STORAGE_DELETE_FILE: &str =
            include_str!("../../deps/interfaces/io.edgehog.devicemanager.storage.DeleteFile.json");
        const STORAGE_FILE: &str =
            include_str!("../../deps/interfaces/io.edgehog.devicemanager.storage.File.json");
        const STORAGE_RESPONSE: &str =
            include_str!("../../deps/interfaces/io.edgehog.devicemanager.storage.Response.json");

        builder = builder
            .interface_str(FILE_TRANSFER_CAPABILITIES)?
            .interface_str(FILE_TRANSFER_DEVICE_TO_SERVER)?
            .interface_str(FILE_TRANSFER_PROGRESS)?
            .interface_str(FILE_TRANSFER_RESPONSE)?
            .interface_str(FILE_TRANSFER_SERVER_TO_DEVICE)?
            .interface_str(STORAGE_DELETE_FILE)?
            .interface_str(STORAGE_FILE)?
            .interface_str(STORAGE_RESPONSE)?;
    }

    Ok(builder)
}
