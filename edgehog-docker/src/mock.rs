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
use std::pin::Pin;

use async_trait::async_trait;
use bollard::{
    auth::DockerCredentials,
    container::{
        AttachContainerOptions, AttachContainerResults, Config, CreateContainerOptions,
        ListContainersOptions, LogOutput, LogsOptions, RemoveContainerOptions,
        StartContainerOptions, Stats, StatsOptions, StopContainerOptions,
    },
    errors::Error,
    image::{CreateImageOptions, RemoveImageOptions},
    models::{ContainerCreateResponse, CreateImageInfo, EventMessage},
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
    async fn create_container<'a, 'b>(
        &self,
        options: Option<CreateContainerOptions<&'a str>>,
        config: Config<&'b str>,
    ) -> Result<ContainerCreateResponse, Error>;
    async fn attach_container<'a>(
        &self,
        container_name: &str,
        options: Option<AttachContainerOptions<&'a str>>,
    ) -> Result<AttachContainerResults, Error>;
    fn create_image(
        &self,
        options: Option<CreateImageOptions<String>>,
        root_fs: Option<Body>,
        credentials: Option<DockerCredentials>,
    ) -> DockerStream<CreateImageInfo>;
    async fn list_containers<'a>(
        &self,
        options: Option<ListContainersOptions<&'a str>>,
    ) -> Result<Vec<ContainerSummary>, Error>;
    fn stats(&self, container_name: &str, options: Option<StatsOptions>) -> DockerStream<Stats>;
    async fn stop_container(
        &self,
        container_name: &str,
        options: Option<StopContainerOptions>,
    ) -> Result<(), Error>;
    fn logs(
        &self,
        container_name: &str,
        options: Option<LogsOptions<String>>,
    ) -> DockerStream<LogOutput>;
    async fn remove_image(
        &self,
        image_name: &str,
        options: Option<RemoveImageOptions>,
        credentials: Option<DockerCredentials>,
    ) -> Result<Vec<ImageDeleteResponseItem>, Error>;
    fn events(&self, options: Option<EventsOptions<&str>>) -> DockerStream<EventMessage>;
    async fn ping(&self) -> Result<String, Error>;
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
        async fn create_container<'a, 'b>(
            &self,
            options: Option<CreateContainerOptions<&'a str>>,
            config: Config<&'b str>,
        ) -> Result<ContainerCreateResponse, Error>;
        async fn attach_container<'a>(
            &self,
            container_name: &str,
            options: Option<AttachContainerOptions<&'a str>>,
        ) -> Result<AttachContainerResults, Error>;
        fn create_image(
            &self,
            options: Option<CreateImageOptions<String>>,
            root_fs: Option<Body>,
            credentials: Option<DockerCredentials>,
        ) -> DockerStream<CreateImageInfo>;
        async fn list_containers<'a>(
            &self,
            options: Option<ListContainersOptions<&'a str>>,
        ) -> Result<Vec<ContainerSummary>, Error>;
        fn stats(&self, container_name: &str, options: Option<StatsOptions>) -> DockerStream<Stats>;
        async fn stop_container(
            &self,
            container_name: &str,
            options: Option<StopContainerOptions>,
        ) -> Result<(), Error>;
        fn logs(
            &self,
            container_name: &str,
            options: Option<LogsOptions<String>>,
        ) -> DockerStream<LogOutput>;
        async fn remove_image(
            &self,
            image_name: &str,
            options: Option<RemoveImageOptions>,
            credentials: Option<DockerCredentials>,
        ) -> Result<Vec<ImageDeleteResponseItem>, Error>;
        fn events<'a>(&self, options: Option<EventsOptions<&'a str>>) -> DockerStream<EventMessage>;
        async fn ping(&self) -> Result<String, Error>;
    }
}
