// This file is part of Edgehog.
//
// Copyright 2023-2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

// NOTE: this is only temporary for making CI happy
#![allow(dead_code)]

use std::marker::Send;
use std::pin::Pin;

use bollard::auth::DockerCredentials;
use bollard::errors::Error;
use bollard::models::NetworkInspect;
use bollard::models::{
    ContainerCreateBody, ContainerCreateResponse, ContainerInspectResponse, ContainerStatsResponse,
    ContainerWaitResponse, CreateImageInfo, EventMessage, ImageInspect, ImageSummary, Network,
    NetworkCreateRequest, NetworkCreateResponse, Volume, VolumeCreateRequest, VolumeListResponse,
};
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptions, EventsOptions, InspectContainerOptions,
    InspectNetworkOptions, ListContainersOptions, ListImagesOptions, ListNetworksOptions,
    ListVolumesOptions, RemoveContainerOptions, RemoveImageOptions, RemoveVolumeOptions,
    StartContainerOptions, StatsOptions, StopContainerOptions, WaitContainerOptions,
};
use bollard::service::{ContainerSummary, ImageDeleteResponseItem};
use futures::Stream;
use hyper::body::Bytes;
use mockall::mock;

type DockerStream<T> = Pin<Box<dyn Stream<Item = Result<T, Error>> + Send>>;
type RootFs = http_body_util::Either<
    http_body_util::Full<Bytes>,
    http_body_util::StreamBody<
        Pin<Box<dyn Stream<Item = Result<http_body::Frame<Bytes>, Error>> + Send>>,
    >,
>;

pub trait DockerTrait: Sized {
    fn connect_with_local_defaults() -> Result<Self, Error>;
    async fn remove_container(
        &self,
        container_name: &str,
        options: Option<RemoveContainerOptions>,
    ) -> Result<(), Error>;
    async fn start_container(
        &self,
        container_name: &str,
        options: Option<StartContainerOptions>,
    ) -> Result<(), Error>;
    async fn create_container(
        &self,
        options: Option<CreateContainerOptions>,
        config: ContainerCreateBody,
    ) -> Result<ContainerCreateResponse, Error>;
    fn create_image(
        &self,
        options: Option<CreateImageOptions>,
        root_fs: Option<RootFs>,
        credentials: Option<DockerCredentials>,
    ) -> DockerStream<CreateImageInfo>;
    async fn list_containers(
        &self,
        options: Option<ListContainersOptions>,
    ) -> Result<Vec<ContainerSummary>, Error>;
    fn stats(
        &self,
        container_name: &str,
        options: Option<StatsOptions>,
    ) -> DockerStream<ContainerStatsResponse>;
    async fn stop_container(
        &self,
        container_name: &str,
        options: Option<StopContainerOptions>,
    ) -> Result<(), Error>;
    async fn remove_image(
        &self,
        image_name: &str,
        options: Option<RemoveImageOptions>,
        credentials: Option<DockerCredentials>,
    ) -> Result<Vec<ImageDeleteResponseItem>, Error>;
    fn events(&self, options: Option<EventsOptions>) -> DockerStream<EventMessage>;
    async fn ping(&self) -> Result<String, Error>;
    async fn inspect_image(&self, image_name: &str) -> Result<ImageInspect, Error>;
    fn wait_container(
        &self,
        container_name: &str,
        options: Option<WaitContainerOptions>,
    ) -> DockerStream<ContainerWaitResponse>;
    async fn list_images(
        &self,
        options: Option<ListImagesOptions>,
    ) -> Result<Vec<ImageSummary>, Error>;
    async fn create_volume(&self, config: VolumeCreateRequest) -> Result<Volume, Error>;
    async fn inspect_volume(&self, volume_name: &str) -> Result<Volume, Error>;
    async fn remove_volume(
        &self,
        image_name: &str,
        options: Option<RemoveVolumeOptions>,
    ) -> Result<(), Error>;
    async fn list_volumes(
        &self,
        options: Option<ListVolumesOptions>,
    ) -> Result<VolumeListResponse, Error>;
    async fn create_network(
        &self,
        config: NetworkCreateRequest,
    ) -> Result<NetworkCreateResponse, Error>;
    async fn inspect_network(
        &self,
        network_name: &str,
        options: Option<InspectNetworkOptions>,
    ) -> Result<NetworkInspect, Error>;
    async fn remove_network(&self, network_name: &str) -> Result<(), Error>;
    async fn list_networks(
        &self,
        options: Option<ListNetworksOptions>,
    ) -> Result<Vec<Network>, Error>;
    async fn inspect_container(
        &self,
        container_name: &str,
        options: Option<InspectContainerOptions>,
    ) -> Result<ContainerInspectResponse, Error>;
}

mock! {
    #[derive(Debug)]
    pub Docker {}
    impl Clone for Docker {
        fn clone(&self) -> Self;
    }
    impl DockerTrait  for Docker {
        fn connect_with_local_defaults() -> Result<Self, Error>;
        async fn remove_container(
            &self,
            container_name: &str,
            options: Option<RemoveContainerOptions>,
        ) -> Result<(), Error>;
        async fn start_container(
            &self,
            container_name: &str,
            options: Option<StartContainerOptions>,
        ) -> Result<(), Error>;
        async fn create_container(
            &self,
            options: Option<CreateContainerOptions>,
            config: ContainerCreateBody,
        ) -> Result<ContainerCreateResponse, Error>;
        fn create_image(
            &self,
            options: Option<CreateImageOptions>,
            root_fs: Option<RootFs>,
            credentials: Option<DockerCredentials>,
        ) -> DockerStream<CreateImageInfo>;
        async fn list_containers(
            &self,
            options: Option<ListContainersOptions>,
        ) -> Result<Vec<ContainerSummary>, Error>;
        fn stats(
            &self,
            container_name: &str,
            options: Option<StatsOptions>,
        ) -> DockerStream<ContainerStatsResponse>;
        async fn stop_container(
            &self,
            container_name: &str,
            options: Option<StopContainerOptions>,
        ) -> Result<(), Error>;
        async fn remove_image(
            &self,
            image_name: &str,
            options: Option<RemoveImageOptions>,
            credentials: Option<DockerCredentials>,
        ) -> Result<Vec<ImageDeleteResponseItem>, Error>;
        fn events(&self, options: Option<EventsOptions>) -> DockerStream<EventMessage>;
        async fn ping(&self) -> Result<String, Error>;
        async fn inspect_image(&self, image_name: &str) -> Result<ImageInspect, Error>;
        fn wait_container(
            &self,
            container_name: &str,
            options: Option<WaitContainerOptions>,
        ) -> DockerStream<ContainerWaitResponse>;
        async fn list_images(
            &self,
            options: Option<ListImagesOptions>,
        ) -> Result<Vec<ImageSummary>, Error>;
        async fn create_volume(&self, config: VolumeCreateRequest) -> Result<Volume, Error>;
        async fn inspect_volume(&self, volume_name: &str) -> Result<Volume, Error>;
        async fn remove_volume(
            &self,
            image_name: &str,
            options: Option<RemoveVolumeOptions>,
        ) -> Result<(), Error>;
        async fn list_volumes(
            &self,
            options: Option<ListVolumesOptions>,
        ) -> Result<VolumeListResponse, Error>;
        async fn create_network(
            &self,
            config: NetworkCreateRequest,
        ) -> Result<NetworkCreateResponse, Error>;
        async fn inspect_network(
            &self,
            network_name: &str,
            options: Option<InspectNetworkOptions>,
        ) -> Result<NetworkInspect, Error>;
        async fn remove_network(&self, network_name: &str) -> Result<(), Error>;
        async fn list_networks(
            &self,
            options: Option<ListNetworksOptions>,
        ) -> Result<Vec<Network>, Error>;
        async fn inspect_container(
            &self,
            container_name: &str,
            options: Option<InspectContainerOptions>,
        ) -> Result<ContainerInspectResponse, Error>;
    }
}
