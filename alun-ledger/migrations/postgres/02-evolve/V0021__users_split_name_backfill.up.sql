UPDATE users SET first_name = split_part(name, ' ', 1), last_name = CASE WHEN position(' ' in name) > 0 THEN substr(name, position(' ' in name) + 1) ELSE '' END
