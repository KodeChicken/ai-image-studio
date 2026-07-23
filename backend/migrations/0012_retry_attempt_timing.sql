UPDATE image_tasks AS task
SET started_at = attempt.started_at
FROM (
    SELECT task_id, MAX(created_at) AS started_at
    FROM task_events
    WHERE to_status = 'processing'
    GROUP BY task_id
) AS attempt
WHERE task.id = attempt.task_id
  AND task.retry_count > 0
  AND task.started_at IS DISTINCT FROM attempt.started_at;
