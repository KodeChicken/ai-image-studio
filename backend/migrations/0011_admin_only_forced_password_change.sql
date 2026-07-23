UPDATE users
SET must_change_password = FALSE,
    updated_at = NOW()
WHERE role = 'user'
  AND must_change_password = TRUE;
