ALTER TABLE deployment_history
    ADD COLUMN source_job_id UUID UNIQUE REFERENCES update_jobs(id) ON DELETE SET NULL;
