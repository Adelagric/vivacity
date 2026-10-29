<?php

namespace ComposerPatcher\Exception;

use ComposerPatcher\Exception;

/**
 * An exception thrown when a command is not found.
 */
class CommandNotFound extends Exception
{
    /**
     * The comamnd that could not be found.
     *
     * @var string
     */
    protected $command;

    /**
     * @param string $command the comamnd that could not be found
     * @param string $message an optional message
     */
    public function __construct($command, $message = '')
    {
        $this->command = $command;
        $message = (string) $message;
        if ($message === '') {
            $message = "Unable to find the command \"{$command}\".";
        }
        parent::__construct($message);
    }

    /**
     * Get the command that could not be found.
     *
     * @return string
     */
    public function getCommand()
    {
        return $this->command;
    }
}
