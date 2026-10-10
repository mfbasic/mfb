Take a loot at the canvas API and native backend...
I would like to do something like this:

--

## Big Change:

* Remove canvas::RoundedRect
* Remove transform from canvas::Paint.
* Remove canvas::Point replace with vector::Float2
* local geometry → shape transform → position → enclosing group transforms

## Add new:

UNION Shape
    Rectangle
    Line
    Polygon
    Circle
    Arc
    Text
    Ellipse
    PolyLine
END UNION

UNION AutoShape INCLUDES Shape
    AutoPaint
END UNION

UNION DrawItem INCLUDES AutoShape
    Picture
    Group
END UNION

' Will update the Shape's paint based on the current mouse movement
' default - Keep the shape’s original paint unchanged
' hover - set when the mouse is within the Shape
' pressed - set when the mouse is within the Shape and a mouse button is pressed
' enter - set when the mouse enters the shape for 50ms then to hover; auto-scheduled redraw when after expires (even if the pointer stops moving)
' exit - set when the mouse exits the shape for 50ms then to default; auto-scheduled redraw when after expires (even if the pointer stops moving)

TYPE AutoPaint
    shape      AS canvas::Shape
    useEnter   AS Boolean
    useExit    AS Boolean
    useHover   AS Boolean
    usePressed AS Boolean
    enter      AS canvas::Paint
    exit       AS canvas::Paint
    hover      AS canvas::Paint
    pressed    AS canvas::Paint
END TYPE

* `pressed` gets priority over `enter` and `hover`

* group interaction. A group saves the DrawItem list. Each use of the group gets its own identity. This mean if a group is used twice, AutoPain applies seteratly to each DrawItem instance.

* Hit-testing on stroke-only shapes. Line, PolyLine, Arc (unfilled) have zero area. Hit region must be stroke width (plus probably a min tolerance), otherwise AutoPaint on a Line is unusable.

* Hit-testing is based on the world shape points after all transforms apply. Not the bounding box, but the final shape. Internally a "fast" bounding box check can heppen as a 1st pass, but the final hit-testing is the actual shape

* Hit-testing is based on the Shape's hit-box. If hover changes the shape, for example, it does not change the hit-box.

## Updates / Cleanups:

ENUM BindTo
    TopLeft
    Center
END ENUM

ENUM PictureClip
    ClipOnly            ' Clip picture to the Shape
    ClipAndTransform    ' Clip picture to the Shape and have the shape transform apply to image
END ENUM

' Contain and Cover center the image

ENUM PictureCover
    Stretch             ' Fill the shape’s local bounds exactly
    Contain             ' Preserve aspect ratio and fit the entire image inside the bounds
    Cover               ' Preserve aspect ratio and fill the bounds completely
END ENUM

TYPE MouseEvent
    kind     AS MouseKind
    button   AS MouseButton
    position AS vector::Float2
    shift    AS Boolean
    ctrl     AS Boolean
    alt      AS Boolean
    overID   AS String
END TYPE

' Clamped radius:
'      radius = clamp(value, 0, min(width / 2, height / 2)).
'      yes for a 1x1 size this make a subpixel error that noone can actually see
'      and I'm okay with that.
' 
' Radius from 0 through 1:
'     Horizontal radius = radius * width / 2.
'     Vertical radius = radius * height / 2.
' 
' Radius greater than 1:
'     Horizontal and vertical radius = min(radius, width / 2, height / 2).

TYPE Rectangle
    id        AS String
    position  AS vector::Float2   ' Based on bind
    bind      AS canvas::BindTo
    size      AS canvas::Size
    radiusTL  AS Float
    radiusTR  AS Float
    radiusBL  AS Float
    radiusBR  AS Float
    paint     AS canvas::Paint
    transform AS canvas::Transform
END TYPE

TYPE Circle
    id        AS String
    position  AS vector::Float2   ' Center
    radius    AS Float
    paint     AS canvas::Paint
    transform AS canvas::Transform
END TYPE

TYPE Ellipse
    id        AS String
    position  AS vector::Float2   ' Center
    radiusX   AS Float
    radiusY   AS Float
    paint     AS canvas::Paint
    transform AS canvas::Transform
END TYPE

TYPE Arc
    id        AS String
    position  AS vector::Float2   ' Center
    radius    AS Float
    startAngle AS Float
    endAngle  AS Float
    paint     AS canvas::Paint
    transform AS canvas::Transform
END TYPE

TYPE Line
    id        AS String
    position  AS vector::Float2   ' Reference Point
    startAt   AS vector::Float2
    endAt     AS vector::Float2
    paint     AS canvas::Paint
    transform AS canvas::Transform
END TYPE

TYPE Text
    id        AS String
    position  AS vector::Float2   ' baseline's leftmost position
    text      AS AttributedString
    font      AS RES canvas::Font
    fontSize  AS Float
    paint     AS canvas::Paint
    transform AS canvas::Transform
END TYPE

' Bigger update, contours are local to the position now
TYPE Polygon
    id        AS String
    position  AS vector::Float2   ' Reference Point
    contours  AS List OF List OF vector::Float2
    rule      AS FillRule
    paint     AS canvas::Paint
    transform AS canvas::Transform
END TYPE

' Bigger update, points are local to the position now
TYPE PolyLine
    id        AS String
    position  AS vector::Float2   ' Reference Point
    points    AS List OF vector::Float2
    colors    AS List OF color::Color
    paint     AS canvas::Paint
    transform AS canvas::Transform
END TYPE

TYPE Picture
    shape     AS canvas::AutoShape      ' This is the shape/position/etc the picture clipped to
    clip      AS canvas::PictureClip
    cover     AS canvas::PictureCover
    image     AS RES canvas::Image
END TYPE

TYPE Group
    transform AS canvas::Transform
    name      AS String
END TYPE

## Future (dont do this now):

### 3D Object in 2D World

' This is drawing a 3d model in a 2D world, flat lighting etc.
' This is not meant to be a full 3D world.

TYPE Mesh
    verticies AS List OF vector::Float3
    triangles AS List OF vector::Integer3
    UV        AS List OF vector::Float2
    normals   AS List OF vector::Float3
    colors    AS List OF color::Color       ' vertex colors
END TYE

TYPE 3DObject
    position  AS vector::Float2   ' Reference Point
    mesh      AS Mesh
    texture   AS REF canvas::Image
    transform AS canvas::Transform
END 3DObject

### Particles - plan 150

ENUM canvas::EmissionMode
    Continuous
    Burst
END ENUM

TYPE canvas::ParticleSystem               ' the 11th DrawItem variant — a CONTAINER, no paint
    position    AS vector::Float2         ' emitter Reference Point
    item        AS canvas::DrawItem       ' the template drawn once per live particle
    emitter     AS canvas::Shape
    emission    AS canvas::Emission
    motion      AS canvas::Motion
    appearance  AS canvas::Appearance
    seed        AS Integer                ' fixes every per-particle random draw
    time        AS Float                  ' the program's clock in seconds; the program advances it
END TYPE

TYPE canvas::Emission
    mode          AS canvas::EmissionMode
    maxParticles  AS Integer   ' the slot pool; the hard bound on this system's quads
    rate          AS Float     ' Continuous: particles per second
    burst         AS Integer   ' Burst: particles per burst, clamped to maxParticles
    burstPeriod   AS Float     ' Burst: seconds between bursts; 0 = a single burst at time 0
    lifetime      AS Float     ' seconds a particle lives
    lifetimeVar   AS Float     ' ± fraction of lifetime, 0..1
END TYPE

TYPE canvas::Motion
    speed         AS Float     ' initial speed, px/s
    speedVar      AS Float     ' ± fraction, 0..1
    angle         AS Float     ' initial direction in radians; 0 = +X, matching Arc
    spread        AS Float     ' ± radians around angle; PI = omnidirectional
    gravityX      AS Float     ' px/s²
    gravityY      AS Float     ' px/s² — positive is DOWN, matching canvas' top-left origin
    drag          AS Float     ' exponential damping, 1/s; 0 = none
    rotation      AS Float     ' initial rotation, radians
    rotationVar   AS Float     ' ± radians
    spin          AS Float     ' radians/s
    spinVar       AS Float     ' ± fraction
    turbulence    AS Float     ' px amplitude of the wander
    turbulenceHz  AS Float     ' cycles/sec of the wander
END TYPE

TYPE canvas::Appearance
    startColor    AS color::Color   ' multiplied into the template's fill AND stroke
    endColor      AS color::Color
    startScale    AS Float
    endScale      AS Float
    scaleVar      AS Float     ' ± fraction, applied to both endpoints per particle
END TYPE
