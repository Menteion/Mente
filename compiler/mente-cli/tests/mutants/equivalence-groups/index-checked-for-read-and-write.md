`xs[i] += 10` checks the index once for the read and again for the write, at the same place and with the same message: i has not changed, so the read's check fails first and the write's never.
