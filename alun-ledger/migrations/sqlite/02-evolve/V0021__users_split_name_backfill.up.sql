UPDATE users SET first_name = substr(name, 1, instr(name || ' ', ' ') - 1), last_name = CASE WHEN instr(name, ' ') > 0 THEN substr(name, instr(name, ' ') + 1) ELSE '' END
