<?php
// Appends one line per event: the event name, COMPOSER_DEV_MODE as seen by
// the process Composer spawned.
file_put_contents('scripts.log', $argv[1] . ' dev=' . getenv('COMPOSER_DEV_MODE') . PHP_EOL, FILE_APPEND);
