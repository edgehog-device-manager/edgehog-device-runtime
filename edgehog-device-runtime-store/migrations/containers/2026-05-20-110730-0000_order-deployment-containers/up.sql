CREATE TABLE IF NOT EXISTS deployment_containers_v2 (
    deployment_id BLOB NOT NULL REFERENCES deployments (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    idx UNSIGNED BIG INT NOT NULL,
    PRIMARY KEY (deployment_id, container_id),
    UNIQUE (deployment_id, idx)
);

INSERT INTO deployment_containers_v2 (
    deployment_id,
    container_id,
    idx
)
SELECT
    deployment_id,
    container_id,
    row_number()
        OVER (
            PARTITION BY deployment_id, container_id
            ORDER BY deployment_id, container_id
        )
    - 1 AS idx
FROM deployment_containers;

CREATE TABLE IF NOT EXISTS deployment_missing_containers_v2 (
    deployment_id BLOB NOT NULL REFERENCES deployments (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    container_id BLOB NOT NULL,
    idx UNSIGNED BIG INT NOT NULL,
    PRIMARY KEY (deployment_id, container_id),
    UNIQUE (deployment_id, idx)
);

INSERT INTO deployment_missing_containers_v2 (
    deployment_id,
    container_id,
    idx
)
SELECT
    deployment_id,
    container_id,
    row_number()
        OVER (
            PARTITION BY deployment_id, container_id
            ORDER BY deployment_id, container_id
        )
    - 1 AS idx
FROM deployment_missing_containers;


DROP TABLE deployment_containers;
ALTER TABLE deployment_containers_v2 RENAME TO deployment_containers;

DROP TABLE deployment_missing_containers;
ALTER TABLE deployment_missing_containers_v2 RENAME TO deployment_missing_containers;
