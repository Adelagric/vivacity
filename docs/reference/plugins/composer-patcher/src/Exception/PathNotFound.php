<?php

namespace ComposerPatcher\Exception;

use ComposerPatcher\Exception;

/**
 * An exception thrown when a path could not be found.
 */
class PathNotFound extends Exception
{
    /**
     * The path that could not be found.
     *
     * @var string
     */
    protected $path;

    /**
     * @param string $path the path that could not be found
     * @param string $message an optional message
     */
    public function __construct($path, $message = '')
    {
        $this->path = $path;
        $message = (string) $message;
        if ($message === '') {
            $message = "Unable to find the path \"{$path}\".";
        }
        parent::__construct($message);
    }

    /**
     * Get the path that could not be found.
     *
     * @return string
     */
    public function getPath()
    {
        return $this->path;
    }
}
