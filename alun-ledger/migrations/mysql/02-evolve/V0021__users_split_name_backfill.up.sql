UPDATE users SET first_name = SUBSTRING_INDEX(name, ' ', 1), last_name = IF(INSTR(name, ' ') > 0, SUBSTRING(name, INSTR(name, ' ') + 1), '')
