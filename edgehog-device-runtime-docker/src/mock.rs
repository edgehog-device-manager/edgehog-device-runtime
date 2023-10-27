// This file is part of Edgehog.
//
// Copyright 2023 SECO Mind Srl
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
use std::marker::Send;
use std::pin::Pin;

use async_trait::async_trait;
use bollard::{
    auth::DockerCredentials,
    container::{
        Config, CreateContainerOptions, ListContainersOptions, LogOutput, LogsOptions,
        RemoveContainerOptions, StartContainerOptions, Stats, StatsOptions, StopContainerOptions,
        WaitContainerOptions,
    },
    errors::Error,
    image::{CreateImageOptions, ListImagesOptions, RemoveImageOptions},
    models::{
        ContainerCreateResponse, ContainerWaitResponse, CreateImageInfo, EventMessage,
        ImageInspect, ImageSummary,
    },
    service::{ContainerSummary, ImageDeleteResponseItem},
    system::EventsOptions,
};
use futures::Stream;
use hyper::Body;
use mockall::mock;

type DockerStream<T> = Pin<Box<dyn Stream<Item = Result<T, Error>> + Send>>;

#[async_trait]
pub trait DockerTrait: Sized {
    fn connect_with_local_defaults() -> Result<Self, Error>;
    async fn remove_container(
        &self,
        container_name: &str,
        options: Option<RemoveContainerOptions>,
    ) -> Result<(), Error>;
    async fn start_container<'a>(
        &self,
        container_name: &str,
        options: Option<StartContainerOptions<&'a str>>,
    ) -> Result<(), Error>;
    async fn create_container<'a>(
        &self,
        options: Option<CreateContainerOptions<&'a str>>,
        caonfig: Config<String>,
    ) -> Result<ContainerCreateResponse, Error>;
    fn create_image(
        &self,
        options: Option<CreateImageOptions<'static, String>>,
        root_fs: Option<Body>,
        credentials: Option<DockerCredentials>,
    ) -> DockerStream<CreateImageInfo>;
    async fn list_containers(
        &self,
        options: Option<ListContainersOptions<String>>,
    ) -> Result<Vec<ContainerSummary>, Error>;
    fn stats(&self, container_name: &str, options: Option<StatsOptions>) -> DockerStream<Stats>;
    async fn stop_container(
        &self,
        container_name: &str,
        options: Option<StopContainerOptions>,
    ) -> Result<(), Error>;
    fn logs<'a>(
        &'a self,
        container_name: &str,
        options: Option<LogsOptions<&'a str>>,
    ) -> DockerStream<LogOutput>;
    async fn remove_image(
        &self,
        image_name: &str,
        options: Option<RemoveImageOptions>,
        credentials: Option<DockerCredentials>,
    ) -> Result<Vec<ImageDeleteResponseItem>, Error>;
    fn events<'a>(&'a self, options: Option<EventsOptions<&'a str>>) -> DockerStream<EventMessage>;
    async fn ping(&self) -> Result<String, Error>;
    async fn inspect_image(&self, image_name: &str) -> Result<ImageInspect, Error>;
    fn wait_container<'a>(
        &'a self,
        container_name: &str,
        options: Option<WaitContainerOptions<&'a str>>,
    ) -> DockerStream<ContainerWaitResponse>;
    async fn list_images(
        &self,
        options: Option<ListImagesOptions<String>>,
    ) -> Result<Vec<ImageSummary>, Error>;
}

mock! {
    #[derive(Debug)]
    pub Docker {}
    impl Clone for Docker {
        fn clone(&self) -> Self;
    }
    #[async_trait]
    impl DockerTrait  for Docker {
        fn connect_with_local_defaults() -> Result<Self, Error>;
        async fn remove_container(
            &self,
            container_name: &str,
            options: Option<RemoveContainerOptions>,
        ) -> Result<(), Error>;
        async fn start_container<'a>(
            &self,
            container_name: &str,
            options: Option<StartContainerOptions<&'a str>>,
        ) -> Result<(), Error>;
        async fn create_container<'a>(
            &self,
            options: Option<CreateContainerOptions<&'a str>>,
            config: Config<String>,
        ) -> Result<ContainerCreateResponse, Error>;
        fn create_image(
            &self,
            options: Option<CreateImageOptions<'static, String>>,
            root_fs: Option<Body>,
            credentials: Option<DockerCredentials>,
        ) -> DockerStream<CreateImageInfo>;
        async fn list_containers(
            &self,
            options: Option<ListContainersOptions<String>>,
        ) -> Result<Vec<ContainerSummary>, Error>;
        fn stats(&self, container_name: &str, options: Option<StatsOptions>) -> DockerStream<Stats>;
        async fn stop_container(
            &self,
            container_name: &str,
            options: Option<StopContainerOptions>,
        ) -> Result<(), Error>;
        fn logs<'a>(
            &'a self,
            container_name: &str,
            options: Option<LogsOptions<&'a str>>,
        ) -> DockerStream<LogOutput>;
        async fn remove_image(
            &self,
            image_name: &str,
            options: Option<RemoveImageOptions>,
            credentials: Option<DockerCredentials>,
        ) -> Result<Vec<ImageDeleteResponseItem>, Error>;
        fn events<'a>(&'a self, options: Option<EventsOptions<&'a str>>) -> DockerStream<EventMessage>;
        async fn ping(&self) -> Result<String, Error>;
        async fn inspect_image(&self, image_name: &str) -> Result<ImageInspect, Error>;
        fn wait_container<'a>(
            &'a self,
            container_name: &str,
            options: Option<WaitContainerOptions<&'a str>>,
        ) -> DockerStream<ContainerWaitResponse>;
        async fn list_images(
            &self,
            options: Option<ListImagesOptions<String>>,
        ) -> Result<Vec<ImageSummary>, Error>;
    }
}
