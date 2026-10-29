<?php

namespace ComposerPatcher\Exception;

use ComposerPatcher\Exception;

/**
 * An exception thrown when a path could not be created.
 */
class PathNotCreated extends Exception
{
    /**
     * The path that was not created.
     *
     * @var string
     */
    protected $path;

    /**
     * @param string $path the path that was not created
     * @param string $message an optional message
     */
    public function __construct($path, $message = '')
    {
        $this->path = $path;
        $message = (string) $message;
        if ($message === '') {
            $message = "Failed to create the path \"{$path}\".";
        }
        parent::__construct($message);
    }

    /**
     * Get the path that was not created.
     *
     * @return string
     */
    public function getPath()
    {
        return $this->path;
    }
}
