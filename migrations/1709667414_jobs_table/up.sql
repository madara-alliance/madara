CREATE TABLE jobs
(
    id          VARCHAR(255) PRIMARY KEY,
    status      VARCHAR(255) NOT NULL,
    external_id VARCHAR(255),
    metadata    JSON,
    created_at  TIMESTAMP    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TIMESTAMP    NOT NULL DEFAULT CURRENT_TIMESTAMP
);

SELECT diesel_manage_updated_at('jobs');