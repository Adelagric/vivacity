<?php

namespace ComposerPatcher;

use Composer\IO\IOInterface;
use Composer\Util\ProcessExecutor;

abstract class PatchExecutor
{
    /**
     * The IOInterface instance to be used for user feedback.
     *
     * @var \Composer\IO\IOInterface
     */
    protected $io;
    /**
     * The ProcessExecutor instance to be used to actually run commands.
     *
     * @var \Composer\Util\ProcessExecutor
     */
    protected $processExecutor;

    /**
     * Initialize the instance.
     *
     * @param \Composer\Util\ProcessExecutor $processExecutor the ProcessExecutor instance to be used to actually run commands
     * @param \Composer\IO\IOInterface $io the IOInterface instance to be used for user feedback
     */
    protected function __construct(ProcessExecutor $processExecutor, IOInterface $io)
    {
        $this->processExecutor = $processExecutor;
        $this->io = $io;
    }

    /**
     * Apply a patch to a base directory.
     *
     * @param \ComposerPatcher\Patch $patch the patch to be applied
     * @param string $baseDirectory the directory where the patch should be applied
     *
     * @throws \ComposerPatcher\Exception\PatchAlreadyApplied if the patch is already applied
     * @throws \ComposerPatcher\Exception\PatchNotApplied if the patch could not be applied
     *
     * @return bool
     */
    public function applyPatch(Patch $patch, $baseDirectory)
    {
        $error = null;
        foreach ($patch->getLevels() as $patchLevel) {
            try {
                $this->applyPatchLevel($patch, $baseDirectory, $patchLevel);

                return;
            } catch (Exception\PatchAlreadyApplied $x) {
                throw $x;
            } catch (Exception\PatchNotApplied $x) {
                if (null === $error) {
                    $error = $x;
                }
            }
        }
        throw $error;
    }

    /**
     * Apply a patch using a specific patch level to a base directory.
     *
     * @param \ComposerPatcher\Patch $patch the patch to be applied
     * @param string $baseDirectory the directory where the patch should be applied
     * @param string $patchLevel the patch level
     *
     * @throws \ComposerPatcher\Exception\PatchAlreadyApplied if the patch is already applied
     * @throws \ComposerPatcher\Exception\PatchNotApplied when the command failed
     */
    abstract protected function applyPatchLevel(Patch $patch, $baseDirectory, $patchLevel);

    /**
     * Run a command and return the exit code.
     *
     * @param string $command The command to be executed
     *
     * @return array First element is the command return code, second element is the standard output, thirg element is the standard error
     */
    protected function run($command)
    {
        $stdOut = '';
        $stdErr = '';
        $stdOut = array();
        $rc = $this->processExecutor->execute($command, $stdOut);
        $stdOut = (string) $stdOut;
        $stdErr = (string) $this->processExecutor->getErrorOutput();

        return array($rc, $stdOut, $stdErr);
    }

    /**
     * Check if a command exists.
     *
     * @param string $commandName the command name
     * @param string $argument the argument to be used to run the command
     *
     * @throws \ComposerPatcher\Exception\CommandNotFound
     */
    protected function checkCommandExists($commandName, $argument = '--version')
    {
        $command = $commandName;
        $argument = (string) $argument;
        if ('' !== $argument) {
            $command .= ' '.$argument;
        }
        list($rc, , $stdErr) = $this->run($command);
        if (0 !== $rc) {
            throw new Exception\CommandNotFound($commandName, $stdErr ?: null);
        }
    }

    /**
     * Escapes a string to be used as a shell argument.
     *
     * Copy of ProcessUtils::escapeArgument() that is deprecated in Symfony 3.3 and removed in Symfony 4.
     *
     * @param string $argument
     *
     * @return string
     *
     * @copyright MIT License - Copyright (c) 2004-2018 Fabien Potencier
     */
    protected function escape($argument)
    {
        // Fix for PHP bug #43784 escapeshellarg removes % from given string
        // Fix for PHP bug #49446 escapeshellarg doesn't work on Windows
        // @see https://bugs.php.net/bug.php?id=43784
        // @see https://bugs.php.net/bug.php?id=49446
        if ('\\' === \DIRECTORY_SEPARATOR) {
            if ('' === $argument) {
                return escapeshellarg($argument);
            }
            $escapedArgument = '';
            $quote = false;
            foreach (preg_split('/(")/', $argument, -1, \PREG_SPLIT_NO_EMPTY | \PREG_SPLIT_DELIM_CAPTURE) as $part) {
                if ('"' === $part) {
                    $escapedArgument .= '\\"';
                } elseif (self::isSurroundedBy($part, '%')) {
                    // Avoid environment variable expansion
                    $escapedArgument .= '^%"'.substr($part, 1, -1).'"^%';
                } else {
                    // escape trailing backslash
                    if ('\\' === substr($part, -1)) {
                        $part .= '\\';
                    }
                    $quote = true;
                    $escapedArgument .= $part;
                }
            }
            if ($quote) {
                $escapedArgument = '"'.$escapedArgument.'"';
            }

            return $escapedArgument;
        }

        return "'".str_replace("'", "'\\''", $argument)."'";
    }

    private static function isSurroundedBy($arg, $char)
    {
        return 2 < \strlen($arg) && $char === $arg[0] && $char === $arg[\strlen($arg) - 1];
    }
}
