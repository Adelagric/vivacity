<?php

namespace App;

use Composer\Script\Event;

final class Hooks
{
    public static function postAutoloadDump(Event $event): void
    {
        self::log('callable:postAutoloadDump', $event);
    }

    public static function postInstall(Event $event): void
    {
        self::log('callable:postInstall', $event);
    }

    private static function log(string $what, Event $event): void
    {
        $flags = $event->getFlags();
        $line = sprintf(
            "%s dev=%s devMode=%s optimize=%s packages=%d\n",
            $what,
            getenv('COMPOSER_DEV_MODE'),
            $event->isDevMode() ? '1' : '0',
            isset($flags['optimize']) ? ($flags['optimize'] ? '1' : '0') : '-',
            count($event->getComposer()->getRepositoryManager()->getLocalRepository()->getPackages())
        );
        file_put_contents('scripts.log', $line, FILE_APPEND);
    }
}
