<?php

namespace ComposerPatcher\Exception;

use ComposerPatcher\Exception;

/**
 * An exception thrown when a path is not writable.
 */
class PathNotWritable extends Exception
{
    /**
     * The path that is not writable.
     *
     * @var string
     */
    protected $path;

    /**
     * @param string $path the path that is not writable
     * @param string $message an optional message
     */
    public function __construct($path, $message = '')
    {
        $this->path = $path;
        $message = (string) $message;
        if ($message === '') {
            $message = "The path \"{$path}\" is not writable.";
        }
        parent::__construct($message);
    }

    /**
     * Get the path that is not writable.
     *
     * @return string
     */
    public function getPath()
    {
        return $this->path;
    }
}
