-- This file should undo anything in `up.sql`
DROP TABLE deployment_containers;
DROP TABLE deployments;
DROP TABLE container_port_bindings;
DROP TABLE container_binds;
DROP TABLE container_env;
DROP TABLE container_missing_images;
DROP TABLE container_missing_networks;
DROP TABLE container_missing_volumes;
DROP TABLE container_volumes;
DROP TABLE container_networks;
DROP TABLE containers;
DROP TABLE volume_driver_opts;
DROP TABLE volumes;
DROP TABLE network_driver_opts;
DROP TABLE networks;
DROP TABLE images;
