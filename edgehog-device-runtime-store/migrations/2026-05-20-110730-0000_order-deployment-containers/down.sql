CREATE TABLE IF NOT EXISTS deployment_containers_old (
    deployment_id BLOB NOT NULL REFERENCES deployments (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    PRIMARY KEY (deployment_id, container_id)
);

INSERT OR IGNORE INTO deployment_containers_old (
    deployment_id,
    container_id
)
SELECT
    deployment_id,
    container_id
FROM deployment_containers;

CREATE TABLE IF NOT EXISTS deployment_missing_containers_old (
    deployment_id BLOB NOT NULL REFERENCES deployments (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    PRIMARY KEY (deployment_id, container_id)
);

INSERT OR IGNORE INTO deployment_missing_containers_old (
    deployment_id,
    container_id
)
SELECT
    deployment_id,
    container_id
FROM deployment_missing_containers;

DROP TABLE deployment_missing_containers;
ALTER TABLE deployment_missing_containers_old RENAME TO deployment_missing_containers;
